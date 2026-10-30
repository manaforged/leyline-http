use std::fmt;

use crate::core::error::{Cause, Error};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Overflow {
    Fail,
    Truncate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cap {
    pub(crate) limit: BodyLimit,
    pub(crate) overflow: Overflow,
}

impl Cap {
    pub(crate) fn fail(limit: BodyLimit) -> Self {
        Self {
            limit,
            overflow: Overflow::Fail,
        }
    }

    pub(crate) fn truncate_at(session: BodyLimit, caller: usize) -> Self {
        Self {
            limit: BodyLimit::session(session.bytes.min(caller)),
            overflow: Overflow::Truncate,
        }
    }

    pub(crate) fn room(self, used: usize) -> usize {
        self.limit.bytes.saturating_sub(used)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyLimit {
    pub(crate) bytes: usize,
}

impl BodyLimit {
    pub(crate) fn session(bytes: usize) -> Self {
        Self { bytes }
    }

    pub(crate) fn error(self) -> Error {
        Error::new(Cause::Body)
            .with_message(self.to_string())
            .with_source(self)
    }

    pub(crate) fn into_io(self) -> std::io::Error {
        std::io::Error::other(self)
    }

    pub(crate) fn of_io(error: &std::io::Error) -> Option<Self> {
        error.get_ref()?.downcast_ref::<Self>().copied()
    }
}

impl fmt::Display for BodyLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "response body exceeds max_body_size ({} bytes)",
            self.bytes
        )
    }
}

impl std::error::Error for BodyLimit {}
