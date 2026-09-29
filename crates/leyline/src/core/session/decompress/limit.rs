use std::fmt;

use crate::core::error::{Error, Kind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyLimit(pub(crate) usize);

impl BodyLimit {
    pub(crate) fn error(self) -> Error {
        Error::new(Kind::Body).with_message(self.to_string())
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
        write!(f, "response body exceeds max_body_size ({} bytes)", self.0)
    }
}

impl std::error::Error for BodyLimit {}
