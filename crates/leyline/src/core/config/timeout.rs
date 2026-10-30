use std::time::Duration;

const DEFAULT_TOTAL: Option<Duration> = Some(Duration::from_secs(300));
const DEFAULT_CONNECT: Option<Duration> = Some(Duration::from_secs(10));
const DEFAULT_ERROR_BODY: Option<Duration> = Some(Duration::from_secs(10));

#[doc = include_str!("../../../docs/timeout.md")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
#[must_use]
pub struct Timeout {
    total: Option<Option<Duration>>,
    connect: Option<Option<Duration>>,
    read: Option<Option<Duration>>,
    response_header: Option<Option<Duration>>,
    body: Option<Option<Duration>>,
    error_body: Option<Option<Duration>>,
}

impl Timeout {
    /// Returns a timeout set with every limit left at its default.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the total time limit for one request, 300 seconds by default; `None` removes it.
    pub fn total(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.total = Some(d.into());
        self
    }

    /// Sets the connect time limit, 10 seconds by default; `None` removes it.
    pub fn connect(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.connect = Some(d.into());
        self
    }

    /// Sets the time limit between reads, none by default.
    pub fn read(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.read = Some(d.into());
        self
    }

    /// Sets the time limit for the response headers to arrive, none by default.
    pub fn response_header(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.response_header = Some(d.into());
        self
    }

    /// Sets the time limit for reading the response body, none by default.
    pub fn body(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.body = Some(d.into());
        self
    }

    /// Sets the time limit for reading an error response body, 10 seconds by default.
    pub fn error_body(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.error_body = Some(d.into());
        self
    }

    pub(crate) fn over(self, base: &Timeout) -> Timeout {
        Timeout {
            total: self.total.or(base.total),
            connect: self.connect.or(base.connect),
            read: self.read.or(base.read),
            response_header: self.response_header.or(base.response_header),
            body: self.body.or(base.body),
            error_body: self.error_body.or(base.error_body),
        }
    }

    pub(crate) fn body_limit(&self) -> Option<Duration> {
        self.body.flatten()
    }

    pub(crate) fn error_body_limit(&self) -> Option<Duration> {
        self.error_body.unwrap_or(DEFAULT_ERROR_BODY)
    }

    pub(crate) fn total_limit(&self) -> Option<Duration> {
        self.total.unwrap_or(DEFAULT_TOTAL)
    }

    pub(crate) fn connect_limit(&self) -> Option<Duration> {
        self.connect.unwrap_or(DEFAULT_CONNECT)
    }

    pub(crate) fn read_limit(&self) -> Option<Duration> {
        self.read.flatten()
    }

    pub(crate) fn response_header_limit(&self) -> Option<Duration> {
        self.response_header.flatten()
    }
}

impl From<Duration> for Timeout {
    fn from(total: Duration) -> Self {
        Self::new().total(total)
    }
}
