use http::StatusCode;

use crate::core::response::is_error_status;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResponseMode {
    Buffered,
    Streamed,
    ErrorPrefix,
}

impl ResponseMode {
    pub(crate) fn new(stream: bool, status_errors: bool) -> Self {
        match (stream, status_errors) {
            (true, _) => Self::Streamed,
            (false, true) => Self::ErrorPrefix,
            (false, false) => Self::Buffered,
        }
    }

    pub(crate) fn reads_incrementally(self) -> bool {
        self != Self::Buffered
    }

    pub(crate) fn keeps_stream(self, status: StatusCode) -> bool {
        match self {
            Self::Buffered => false,
            Self::Streamed => true,
            Self::ErrorPrefix => is_error_status(status),
        }
    }
}
