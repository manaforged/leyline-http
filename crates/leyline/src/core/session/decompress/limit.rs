use std::fmt;

use crate::core::error::{Error, Kind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LimitSource {
    Session,
    Caller,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyLimit {
    pub(crate) bytes: usize,
    pub(crate) source: LimitSource,
}

impl BodyLimit {
    pub(crate) fn session(bytes: usize) -> Self {
        Self {
            bytes,
            source: LimitSource::Session,
        }
    }

    pub(crate) fn caller(bytes: usize) -> Self {
        Self {
            bytes,
            source: LimitSource::Caller,
        }
    }

    pub(crate) fn tighter(self, caller: Option<u64>) -> Self {
        match caller.map(|bytes| usize::try_from(bytes).unwrap_or(usize::MAX)) {
            Some(bytes) if bytes <= self.bytes => Self::caller(bytes),
            _ => self,
        }
    }

    pub(crate) fn error(self) -> Error {
        Error::new(Kind::Body)
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
        match self.source {
            LimitSource::Session => write!(
                f,
                "response body exceeds max_body_size ({} bytes)",
                self.bytes
            ),
            LimitSource::Caller => write!(
                f,
                "response body exceeds the caller's limit ({} bytes)",
                self.bytes
            ),
        }
    }
}

impl std::error::Error for BodyLimit {}
