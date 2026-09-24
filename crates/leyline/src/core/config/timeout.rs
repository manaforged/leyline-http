use std::time::Duration;

const DEFAULT_TOTAL: Option<Duration> = Some(Duration::from_secs(300));
const DEFAULT_CONNECT: Option<Duration> = Some(Duration::from_secs(10));

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TimeoutConfig {
    total: Option<Option<Duration>>,
    connect: Option<Option<Duration>>,
    read: Option<Option<Duration>>,
    response_header: Option<Option<Duration>>,
}

impl TimeoutConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn total(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.total = Some(d.into());
        self
    }

    pub fn connect(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.connect = Some(d.into());
        self
    }

    pub fn read(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.read = Some(d.into());
        self
    }

    pub fn response_header(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.response_header = Some(d.into());
        self
    }

    pub(crate) fn over(self, base: &TimeoutConfig) -> TimeoutConfig {
        TimeoutConfig {
            total: self.total.or(base.total),
            connect: self.connect.or(base.connect),
            read: self.read.or(base.read),
            response_header: self.response_header.or(base.response_header),
        }
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

impl From<Duration> for TimeoutConfig {
    fn from(total: Duration) -> Self {
        Self::new().total(total)
    }
}
