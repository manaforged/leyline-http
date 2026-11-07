use std::fmt;

use super::{Cause, Error, Kind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyLimit {
    bytes: usize,
}

impl BodyLimit {
    pub(crate) fn session(bytes: usize) -> Self {
        Self { bytes }
    }

    pub(crate) fn admit(self, seen: usize, n: usize) -> Result<(), Self> {
        if seen.saturating_add(n) > self.bytes {
            Err(self)
        } else {
            Ok(())
        }
    }

    pub(crate) fn room(self, used: usize) -> usize {
        self.bytes.saturating_sub(used)
    }

    pub(crate) fn at_most(self, bytes: usize) -> Self {
        Self {
            bytes: self.bytes.min(bytes),
        }
    }

    pub(crate) fn error(self) -> Error {
        Error::new(Kind::Body)
            .with_message(self.to_string())
            .with_cause(self)
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

impl Cause for BodyLimit {
    fn duplicate(&self) -> Option<Box<dyn Cause>> {
        Some(Box::new(*self))
    }
}
