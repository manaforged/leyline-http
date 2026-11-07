use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use http::{Method, StatusCode, header};

use super::{Draft, RequestBuilder};
use crate::core::retry::{Retry, RetryPlan};
use crate::core::session::Session;
use crate::error::{Error, Kind, Result};
use crate::wire::Body;

mod parser;
use parser::Parser;

const DEFAULT_EVENT_BYTES: usize = 1024 * 1024;
const RECONNECT_WAIT: Duration = Duration::from_secs(3);
const RECONNECT_LIMIT: u32 = 3;

/// A server-sent event with its persistent id, event type, and UTF-8 data.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Event {
    id: String,
    event: String,
    data: String,
}

impl Event {
    /// Returns the last event id, including an inherited or explicitly empty id.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Returns the event type, or `message` when none was supplied.
    pub fn event(&self) -> &str {
        &self.event
    }
    /// Returns the event's UTF-8 data with data lines joined by newlines.
    pub fn data(&self) -> &str {
        &self.data
    }
    /// Deserializes the event data as JSON.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_str(&self.data).map_err(Error::from_json)
    }
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Event")
            .field("id_bytes", &self.id.len())
            .field("event_bytes", &self.event.len())
            .field("data_bytes", &self.data.len())
            .finish()
    }
}

/// A lazy, bounded-reconnecting stream of server-sent events.
#[must_use = "events are lazy: await next() or poll the stream"]
pub struct Events {
    state: Mutex<State>,
    settings: std::sync::Arc<Mutex<Settings>>,
}

struct Settings {
    retry: Retry,
    max: usize,
}

type Step = (Option<Result<Event>>, Option<Box<Source>>);
enum State {
    Ready(Box<Source>),
    Polling(Pin<Box<dyn Future<Output = Step> + Send>>),
    Done,
}

struct Source {
    session: Session,
    request: Box<Draft>,
    body: Option<Body>,
    bytes: Bytes,
    parser: Parser,
    settings: std::sync::Arc<Mutex<Settings>>,
    reconnects: u32,
}

impl RequestBuilder {
    /// Opens a lazy GET event stream that reconnects up to three times in a row with Last-Event-ID recovery.
    pub fn events(mut self) -> Result<Events> {
        if self.draft.method != Method::GET || self.draft.body.len_hint() != Some(0) {
            return Err(Error::config(
                "events requires a GET request without a body",
            ));
        }
        self.prepare_in_place()?;
        self.draft.carry.stream_response = true;
        self.draft
            .headers
            .set(header::ACCEPT, "text/event-stream")?;
        self.draft.headers.set(header::CACHE_CONTROL, "no-cache")?;
        let id = self
            .draft
            .headers
            .get("last-event-id")
            .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
            .unwrap_or_default();
        let settings = std::sync::Arc::new(Mutex::new(Settings {
            retry: Retry::new(RECONNECT_LIMIT),
            max: DEFAULT_EVENT_BYTES,
        }));
        Ok(Events {
            settings: std::sync::Arc::clone(&settings),
            state: Mutex::new(State::Ready(Box::new(Source {
                session: self.session,
                request: self.draft,
                body: None,
                bytes: Bytes::new(),
                parser: Parser::new(id, DEFAULT_EVENT_BYTES, RECONNECT_WAIT),
                settings,
                reconnects: 0,
            }))),
        })
    }
}

impl Events {
    /// Sets the reconnect policy and its maximum wait; each delivered event resets the reconnect count.
    pub fn retry(self, retry: impl Into<Retry>) -> Self {
        crate::util::lock(&self.settings).retry = retry.into();
        self
    }
    /// Caps the bytes held for one event; the default is one MiB.
    pub fn max_event_size(self, bytes: usize) -> Self {
        crate::util::lock(&self.settings).max = bytes;
        self
    }
    /// Returns the next event, a terminal error, or `None` after the stream stops.
    pub async fn next(&mut self) -> Option<Result<Event>> {
        std::future::poll_fn(|cx| Pin::new(&mut *self).poll_next(cx)).await
    }
}

impl Stream for Events {
    type Item = Result<Event>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let state = self
            .get_mut()
            .state
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        let mut future = match std::mem::replace(state, State::Done) {
            State::Ready(source) => {
                Box::pin(source.advance()) as Pin<Box<dyn Future<Output = Step> + Send>>
            }
            State::Polling(future) => future,
            State::Done => return Poll::Ready(None),
        };
        match future.as_mut().poll(cx) {
            Poll::Pending => {
                *state = State::Polling(future);
                Poll::Pending
            }
            Poll::Ready((event, source)) => {
                *state = source.map_or(State::Done, State::Ready);
                Poll::Ready(event)
            }
        }
    }
}

impl std::fmt::Debug for Events {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let settings = crate::util::lock(&self.settings);
        f.debug_struct("Events")
            .field("max_event_size", &settings.max)
            .field("retry", &settings.retry)
            .finish_non_exhaustive()
    }
}

impl Source {
    async fn advance(mut self: Box<Self>) -> Step {
        loop {
            match self.read().await {
                Ok(Some(event)) => {
                    self.reconnects = 0;
                    return (Some(Ok(event)), Some(self));
                }
                Ok(None) => return (None, None),
                Err(error) => {
                    if let Err(error) = self.reconnect(error).await {
                        return (Some(Err(error)), None);
                    }
                }
            }
        }
    }

    async fn connect(&mut self) -> Result<bool> {
        let mut draft = self
            .request
            .again(Body::default(), self.request.headers.clone());
        if self.parser.last_id.is_empty() {
            draft.headers.remove_all("last-event-id");
        } else {
            draft
                .headers
                .set("last-event-id", self.parser.last_id.as_str())?;
        }
        let response = RequestBuilder {
            session: self.session.clone(),
            draft,
        }
        .send()
        .await?;
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(false);
        }
        response.error_for_status()?;
        if response.status() != StatusCode::OK
            || !response
                .media_type()
                .is_some_and(|media| media.eq_ignore_ascii_case("text/event-stream"))
        {
            return Err(Error::new(Kind::Request)
                .with_message("SSE response is not text/event-stream with status 200"));
        }
        self.body = Some(response.bytes_stream()?);
        Ok(true)
    }

    async fn read(&mut self) -> Result<Option<Event>> {
        if self.body.is_none() && !self.connect().await? {
            return Ok(None);
        }
        loop {
            self.parser.max = crate::util::lock(&self.settings).max;
            if let Some(event) = self.parser.read(&mut self.bytes)? {
                return Ok(Some(event));
            }
            match self
                .body
                .as_mut()
                .expect("invariant: connect installs the event body")
                .next()
                .await
            {
                Some(chunk) => self.bytes = chunk?,
                None => return Err(Error::new(Kind::Connect).with_message("event stream closed")),
            }
        }
    }

    async fn reconnect(&mut self, error: Error) -> Result<()> {
        let result = Err(error);
        let retry = crate::util::lock(&self.settings).retry.clone();
        let plan = retry.plan(&result, self.reconnects);
        let RetryPlan::Backoff(backoff) = plan else {
            return result.map(|_: crate::core::response::Response| ());
        };
        let wait = self.parser.wait.max(backoff);
        if !retry.allows_wait(wait) {
            return result.map(|_: crate::core::response::Response| ());
        }
        let Some(at) = tokio::time::Instant::now().checked_add(wait) else {
            return result
                .map(|_: crate::core::response::Response| ())
                .map_err(|error| {
                    error.with_message("event reconnect wait exceeds the supported clock range")
                });
        };
        self.body = None;
        self.bytes = Bytes::new();
        self.parser.reset();
        self.reconnects += 1;
        self.session
            .unless_shut_down(async {
                tokio::time::sleep_until(at).await;
                Ok(())
            })
            .await
    }
}
