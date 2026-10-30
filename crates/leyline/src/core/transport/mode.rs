use std::time::Duration;

use crate::core::response::is_error_status;

/// How long and how many bytes to read from an error response body before returning it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ErrorBudget {
    pub(crate) wait: Duration,
    pub(crate) bytes: usize,
}

impl ErrorBudget {
    /// Returns a budget of `wait` time and `bytes` bytes.
    #[must_use]
    pub fn new(wait: Duration, bytes: usize) -> Self {
        Self { wait, bytes }
    }
}

/// How a transport returns a response body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResponseMode {
    /// Reads the whole body before returning.
    Buffered,
    /// Returns at the head and streams the body.
    Streamed,
    /// Streams a success body and reads a prefix of an error body within the budget.
    ErrorPrefix(ErrorBudget),
}

impl ResponseMode {
    pub(crate) fn new(stream: bool, error_budget: Option<ErrorBudget>) -> Self {
        match (stream, error_budget) {
            (true, _) => Self::Streamed,
            (false, Some(budget)) => Self::ErrorPrefix(budget),
            (false, None) => Self::Buffered,
        }
    }

    pub(crate) fn keeps_stream(self, status: u16) -> bool {
        match self {
            Self::Buffered => false,
            Self::Streamed => true,
            Self::ErrorPrefix(_) => is_error_status(status),
        }
    }

    pub(crate) fn error_budget(self) -> Option<ErrorBudget> {
        match self {
            Self::ErrorPrefix(budget) => Some(budget),
            Self::Buffered | Self::Streamed => None,
        }
    }
}

impl From<bool> for ResponseMode {
    fn from(stream: bool) -> Self {
        Self::new(stream, None)
    }
}
