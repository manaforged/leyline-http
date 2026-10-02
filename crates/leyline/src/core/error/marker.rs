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

#[derive(Debug)]
#[cfg_attr(not(feature = "http3"), expect(dead_code))]
pub(crate) struct NotProcessed;

impl fmt::Display for NotProcessed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the server did not process the request")
    }
}

impl std::error::Error for NotProcessed {}

const SHUT_DOWN_MESSAGE: &str = "session shut down";

impl Error {
    pub(crate) fn shut_down() -> Error {
        Error::new(Kind::Request)
            .with_message(SHUT_DOWN_MESSAGE)
            .with_source(ShutDown)
    }

    pub fn is_shut_down(&self) -> bool {
        self.source_as::<ShutDown>().is_some()
    }

    pub(crate) fn not_processed(&self) -> bool {
        self.source_as::<NotProcessed>().is_some()
    }
}
