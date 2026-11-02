use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::Stream;

use super::route::Route;
use super::{Draft, RequestBuilder};
use crate::core::Result;
use crate::core::body::{Body, BodyKind, BoxedStream};
use crate::core::deadline::Deadline;
use crate::core::error::{Error, Kind};
use crate::core::headers::HeaderList;
use crate::core::response::Response;
use crate::core::retry::{Retry, RetryPlan};
use crate::core::session::Session;
use crate::core::transport::ResponseMode;

impl RequestBuilder {
    /// Sends the request with retries and waits for the response headers.
    pub fn send(self) -> impl Future<Output = Result<Response>> + Send {
        self.run()
    }

    fn run(self) -> Pin<Box<dyn Future<Output = Result<Response>> + Send>> {
        match self.respond() {
            Ok(respond) => Box::pin(respond),
            Err(error) => Box::pin(std::future::ready(Err(error))),
        }
    }

    fn respond(mut self) -> Result<impl Future<Output = Result<Response>> + Send + 'static> {
        let url = self.prepare()?;
        let retry_policy = std::mem::take(&mut self.draft.retry_policy);
        let deadline = self.session.deadline(self.draft.timeouts.as_ref());
        let label = self.draft.label.take();
        let status_errors = self.draft.status_errors;
        let streamed = self.draft.stream_response;
        let (session, attempt) = self.into_attempt(deadline, Arc::clone(&url));
        Ok(async move {
            let method = attempt.method.clone();
            let mut exhausted = false;
            let attempts = run_attempts(
                &session,
                attempt,
                &url,
                &retry_policy,
                &deadline,
                label,
                &mut exhausted,
            );
            let response = if session.traces() {
                session
                    .traced(method.as_str(), url.as_str(), streamed, attempts)
                    .await?
            } else {
                attempts.await.0?
            };
            if !status_errors {
                return Ok(response);
            }
            let policy_wait = retry_policy.server_wait(&response);
            response
                .error_for_status_with_body(&deadline)
                .await
                .map_err(|mut error| {
                    error.set_retry_outcome(policy_wait, exhausted);
                    error
                })
        })
    }

    fn into_attempt(self, deadline: Deadline, url: Arc<url::Url>) -> (Session, Box<Draft>) {
        let Self { session, mut draft } = self;
        draft.response = ResponseMode::new(
            draft.stream_response
                || draft.body_rules.max_body_size > session.body_rules().max_body_size,
            draft
                .status_errors
                .then(|| Session::error_budget(&deadline, &draft.body_rules)),
        );
        draft.url = Some(url);
        draft.deadline = deadline;
        (session, draft)
    }

    pub(crate) fn prepare(&mut self) -> Result<Arc<url::Url>> {
        if let Some(err) = self.draft.builder_error.take() {
            return Err(err);
        }
        let Some(mut url) = self.draft.url.take() else {
            return Err(Error::from(url::ParseError::EmptyHost));
        };
        self.infer_from_content_type();

        if !self.draft.query_params.is_empty() {
            let mut pairs = Arc::make_mut(&mut url).query_pairs_mut();
            for (k, v) in &self.draft.query_params {
                pairs.append_pair(k, v);
            }
        }

        if let Some(encoding) = self.draft.compress {
            match &self.draft.body.0 {
                BodyKind::Bytes(b) => {
                    let compressed = encoding.encode(b)?;
                    self.draft
                        .headers
                        .set("content-encoding", encoding.header_value())?;
                    self.draft.body = Body::from(compressed);
                }
                BodyKind::Empty => {}
                BodyKind::Stream { .. } => {
                    return Err(Error::new(Kind::Body).with_message(
                        "request-body compression is not supported for streaming bodies; \
                         buffer the body into bytes before calling `.compress_body(..)`",
                    ));
                }
            }
        }
        Ok(url)
    }
}

async fn run_attempts(
    session: &crate::core::session::Session,
    mut attempt: Box<Draft>,
    url: &Arc<url::Url>,
    retry_policy: &Retry,
    deadline: &Deadline,
    label: Option<String>,
    exhausted: &mut bool,
) -> (Result<Response>, u32) {
    crate::trace::note_label(label);
    let mut route = Route::new(session, Arc::clone(url), &attempt, retry_policy);
    if retry_policy.is_none() {
        return (counted(route.send(attempt).await, 1), 1);
    }
    let retryable_method = retry_policy.allows(attempt.method.as_str());
    let replay = attempt.body.replay();
    let mut rewind = Rewind::wrap(&mut attempt.body);
    let base_headers = attempt.headers.clone();

    let mut n: u32 = 0;
    loop {
        if let Err(err) = deadline.check() {
            return (Err(err), n);
        }
        let attempt_body = std::mem::take(&mut attempt.body);
        let this = attempt.again(attempt_body, base_headers.clone());
        let result = route.send(this).await;

        let body_retryable = || replay.is_some() || rewind.as_mut().is_some_and(Rewind::reclaim);
        let sleep = match retry_policy.plan(&result, n, retryable_method, body_retryable) {
            RetryPlan::Backoff(sleep) if sleep < deadline.remaining() => sleep,
            plan => {
                *exhausted = matches!(
                    plan,
                    RetryPlan::Exhausted | RetryPlan::Declined | RetryPlan::Backoff(_)
                );
                return (counted(result, n + 1), n + 1);
            }
        };
        drop(result);

        let slept = session
            .unless_shut_down(async {
                deadline.sleep(sleep).await;
                Ok(())
            })
            .await;
        if let Err(err) = slept {
            return (counted(Err(err), n + 1), n + 1);
        }
        n += 1;
        attempt.body = replay
            .as_ref()
            .and_then(Body::replay)
            .or_else(|| rewind.as_mut().and_then(Rewind::again))
            .unwrap_or_default();
    }
}

impl Draft {
    fn again(&self, body: Body, headers: HeaderList) -> Box<Draft> {
        Box::new(Draft {
            method: self.method.clone(),
            url: self.url.clone(),
            preset: self.preset,
            body,
            headers,
            query_params: Vec::new(),
            timeouts: None,
            builder_error: None,
            stream_response: self.stream_response,
            compress: None,
            retry_policy: Retry::default(),
            digest_auth: self.digest_auth.clone(),
            proxy: self.proxy.clone(),
            header_order: self.header_order.clone(),
            preset_user: self.preset_user,
            redirect: self.redirect.clone(),
            initiator: self.initiator.clone(),
            trusted_origin: self.trusted_origin.clone(),
            label: None,
            status_errors: self.status_errors,
            body_rules: self.body_rules,
            deadline: self.deadline,
            response: self.response,
        })
    }
}

fn counted(result: Result<Response>, attempts: u32) -> Result<Response> {
    result
        .map(|mut response| {
            response.set_attempts(attempts);
            response
        })
        .map_err(|mut error| {
            error.set_attempts(attempts);
            error
        })
}

struct Slot {
    stream: Option<BoxedStream>,
    polled: bool,
}

type SharedSlot = Arc<Mutex<Slot>>;

struct Rewind {
    slot: SharedSlot,
    length_hint: Option<u64>,
    held: Option<BoxedStream>,
}

impl Rewind {
    fn wrap(body: &mut Body) -> Option<Self> {
        if !matches!(body.0, BodyKind::Stream { .. }) {
            return None;
        }
        let BodyKind::Stream {
            stream,
            length_hint,
        } = std::mem::take(&mut body.0)
        else {
            return None;
        };
        let rewind = Self {
            slot: Arc::new(Mutex::new(Slot {
                stream: Some(stream),
                polled: false,
            })),
            length_hint,
            held: None,
        };
        *body = rewind.body();
        Some(rewind)
    }

    fn body(&self) -> Body {
        Body(BodyKind::Stream {
            stream: Box::pin(Reclaim(Arc::clone(&self.slot))),
            length_hint: self.length_hint,
        })
    }

    fn reclaim(&mut self) -> bool {
        let mut slot = crate::util::lock(&self.slot);
        if slot.polled {
            return false;
        }
        self.held = slot.stream.take();
        self.held.is_some()
    }

    fn again(&mut self) -> Option<Body> {
        let stream = self.held.take()?;
        self.slot = Arc::new(Mutex::new(Slot {
            stream: Some(stream),
            polled: false,
        }));
        Some(self.body())
    }
}

struct Reclaim(SharedSlot);

impl Stream for Reclaim {
    type Item = std::io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut slot = crate::util::lock(&self.0);
        slot.polled = true;
        match slot.stream.as_mut() {
            Some(stream) => stream.as_mut().poll_next(cx),
            None => Poll::Ready(None),
        }
    }
}

#[cfg(test)]
#[path = "send_tests.rs"]
mod tests;
