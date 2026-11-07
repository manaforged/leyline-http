use std::fmt;

use super::{Error, Kind};

#[derive(Debug)]
pub(crate) struct ShutDown;

impl fmt::Display for ShutDown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(SHUT_DOWN_MESSAGE)
    }
}

impl std::error::Error for ShutDown {}

const SHUT_DOWN_MESSAGE: &str = "session shut down";

impl Error {
    pub(crate) fn shut_down() -> Error {
        Error::new(Kind::Request)
            .with_message(SHUT_DOWN_MESSAGE)
            .with_source(ShutDown)
    }

    /// Returns true when the request failed because the session shut down.
    pub fn is_shut_down(&self) -> bool {
        self.source_as::<ShutDown>().is_some()
    }
}
