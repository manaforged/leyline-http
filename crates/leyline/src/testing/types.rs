use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const QUEUE_EMPTY_STATUS: u16 = 503;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RecordedRequest {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RecordedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TestResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub(super) delay: Duration,
    pub(super) chunks: Option<ChunkedBody>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ChunkedBody {
    pub(super) parts: Vec<Vec<u8>>,
    pub(super) pause: Duration,
}

impl Default for TestResponse {
    fn default() -> Self {
        Self::new(200)
    }
}

impl TestResponse {
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            delay: Duration::ZERO,
            chunks: None,
        }
    }

    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn chunks<I, B>(mut self, parts: I, pause: Duration) -> Self
    where
        I: IntoIterator<Item = B>,
        B: Into<Vec<u8>>,
    {
        let parts = parts.into_iter().map(Into::into).collect();
        self.chunks = Some(ChunkedBody { parts, pause });
        self
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }
}

pub type Handler = Arc<dyn Fn(&RecordedRequest) -> TestResponse + Send + Sync>;

pub fn queue(
    responses: impl IntoIterator<Item = TestResponse>,
) -> impl Fn(&RecordedRequest) -> TestResponse + Send + Sync + 'static {
    let pending = Mutex::new(responses.into_iter().collect::<VecDeque<_>>());
    move |_| {
        pending
            .lock()
            .ok()
            .and_then(|mut queue| queue.pop_front())
            .unwrap_or_else(|| TestResponse::new(QUEUE_EMPTY_STATUS))
    }
}

#[derive(Clone)]
pub(super) struct Recorder {
    pub(super) log: Arc<Mutex<Vec<RecordedRequest>>>,
    pub(super) sender: tokio::sync::mpsc::UnboundedSender<RecordedRequest>,
}

impl Recorder {
    pub(super) fn record(&self, request: RecordedRequest) -> bool {
        if let Ok(mut log) = self.log.lock() {
            log.push(request.clone());
        }
        self.sender.send(request).is_ok()
    }

    pub(super) fn snapshot(&self) -> Vec<RecordedRequest> {
        self.log.lock().map(|log| log.clone()).unwrap_or_default()
    }
}
