use std::io;

use super::BodyLimit;
use super::{Error, Kind};

impl Error {
    pub(crate) fn from_json(e: serde_json::Error) -> Self {
        Error::new(Kind::Json).with_source(e)
    }

    pub(crate) fn from_request_body(e: io::Error) -> Self {
        Error::new(Kind::Body)
            .with_message("request body stream failed")
            .with_source(e)
    }
}

impl From<url::ParseError> for Error {
    fn from(e: url::ParseError) -> Self {
        Error::new(Kind::Url).with_source(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        if let Some(limit) = BodyLimit::of_io(&e) {
            return limit.error();
        }
        if e.kind() == io::ErrorKind::TimedOut {
            return Error::new(Kind::Timeout).with_source(e);
        }
        if !e.get_ref().is_some_and(|inner| inner.is::<Error>()) {
            return Error::new(Kind::Io).with_source(e);
        }
        match e.into_inner().map(|inner| inner.downcast::<Error>()) {
            Some(Ok(err)) => *err,
            Some(Err(other)) => Error::new(Kind::Io).with_source(io::Error::other(other)),
            None => Error::new(Kind::Io),
        }
    }
}

impl From<http::Error> for Error {
    fn from(e: http::Error) -> Self {
        Error::new(Kind::Request).with_source(e)
    }
}

impl From<http::header::InvalidHeaderName> for Error {
    fn from(e: http::header::InvalidHeaderName) -> Self {
        Error::new(Kind::Request).with_source(e)
    }
}

impl From<http::header::InvalidHeaderValue> for Error {
    fn from(e: http::header::InvalidHeaderValue) -> Self {
        Error::new(Kind::Request).with_source(e)
    }
}

impl From<http::uri::InvalidUri> for Error {
    fn from(e: http::uri::InvalidUri) -> Self {
        Error::new(Kind::Url).with_source(e)
    }
}
