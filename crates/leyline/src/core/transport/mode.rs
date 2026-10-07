use std::time::Duration;

use crate::core::response::is_error_status;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ErrorBudget {
    pub(crate) wait: Duration,
    pub(crate) bytes: usize,
}

impl ErrorBudget {
    #[must_use]
    pub fn new(wait: Duration, bytes: usize) -> Self {
        Self { wait, bytes }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseMode {
    Buffered,
    Streamed,
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
