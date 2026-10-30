use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const QUEUE_EMPTY_STATUS: u16 = 503;
pub(super) const CONNECTION: &str = "connection";
pub(super) const CLOSE: &str = "close";

/// A request the test server received.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Request {
    /// The request method.
    pub method: String,
    /// The request target as sent, such as `/path?query`.
    pub target: String,
    /// The request headers in the order received.
    pub headers: Vec<(String, String)>,
    /// The request body after any chunked decoding.
    pub body: Vec<u8>,
    /// The request line, such as `GET /path HTTP/1.1`.
    pub request_line: String,
    /// The request line and headers as received, without the body.
    pub raw: Vec<u8>,
}

impl Request {
    /// Returns the first value of the named header, ignoring case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.header_values(name).into_iter().next()
    }

    /// Returns every value of the named header, ignoring case.
    pub fn header_values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect()
    }

    /// Returns the request line and headers as text, with invalid UTF-8 replaced.
    pub fn text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.raw)
    }
}

/// A response the test server sends, built with `Response::new` and its setters.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Response {
    pub(super) status: u16,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
    pub(super) delay: Duration,
    pub(super) chunks: Option<ChunkedBody>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ChunkedBody {
    pub(super) parts: Vec<Vec<u8>>,
    pub(super) pause: Duration,
}

impl Default for Response {
    fn default() -> Self {
        Self::new(200)
    }
}

impl Response {
    /// Returns a response with the given status, no headers, and an empty body.
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            delay: Duration::ZERO,
            chunks: None,
        }
    }

    /// Waits this long before sending the response.
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// Sends a chunked body with `pause` between parts, in place of `body`.
    pub fn chunks<I, B>(mut self, parts: I, pause: Duration) -> Self
    where
        I: IntoIterator<Item = B>,
        B: Into<Vec<u8>>,
    {
        let parts = parts.into_iter().map(Into::into).collect();
        self.chunks = Some(ChunkedBody { parts, pause });
        self
    }

    /// Adds a response header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Adds `connection: close`, so the server closes the connection after this response.
    pub fn close(self) -> Self {
        self.header(CONNECTION, CLOSE)
    }

    pub(super) fn closes(&self) -> bool {
        self.headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case(CONNECTION) && value.eq_ignore_ascii_case(CLOSE)
        })
    }

    /// Sets the response body.
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }
}

pub(crate) type Handler = Arc<dyn Fn(&Request) -> Response + Send + Sync>;

/// Returns a handler that answers each request with the next response from `responses`, then 503.
pub fn queue(
    responses: impl IntoIterator<Item = Response>,
) -> impl Fn(&Request) -> Response + Send + Sync + 'static {
    let pending = Mutex::new(responses.into_iter().collect::<VecDeque<_>>());
    move |_| {
        pending
            .lock()
            .ok()
            .and_then(|mut queue| queue.pop_front())
            .unwrap_or_else(|| Response::new(QUEUE_EMPTY_STATUS))
    }
}

#[derive(Clone)]
pub(super) struct Recorder {
    pub(super) log: Arc<Mutex<Vec<Request>>>,
    pub(super) sender: tokio::sync::mpsc::UnboundedSender<Request>,
}

impl Recorder {
    pub(super) fn record(&self, request: Request) -> bool {
        if let Ok(mut log) = self.log.lock() {
            log.push(request.clone());
        }
        self.sender.send(request).is_ok()
    }

    pub(super) fn snapshot(&self) -> Vec<Request> {
        self.log.lock().map(|log| log.clone()).unwrap_or_default()
    }
}
