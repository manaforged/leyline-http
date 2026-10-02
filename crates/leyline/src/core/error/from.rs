use std::io;

use super::{Error, Kind};
use crate::h2::H2Error;
#[cfg(feature = "http3")]
use crate::quic::H3SendError;
use crate::tls::TlsError;

impl Error {
    pub(crate) fn from_url_parse(e: url::ParseError) -> Self {
        Error::new(Kind::Url).with_source(e)
    }

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
        Error::from_url_parse(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
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

impl From<TlsError> for Error {
    fn from(e: TlsError) -> Self {
        let kind = match &e {
            TlsError::Dns(_) | TlsError::TcpConnect(_) => Kind::Connect,
            TlsError::Proxy { .. } | TlsError::ProxyTargetUnreachable { .. } => Kind::Proxy,
            TlsError::SslConfig(_) | TlsError::Profile(_) | TlsError::TrustStore(_) => Kind::Config,
            _ => Kind::Tls,
        };
        Error::new(kind).with_source(e)
    }
}

impl From<H2Error> for Error {
    fn from(e: H2Error) -> Self {
        if let Some(limit) = e.body_limit() {
            return limit.error();
        }
        match e {
            H2Error::RequestBody(io) => Error::from_request_body(io),
            other => Error::new(Kind::Http2).with_source(other),
        }
    }
}

#[cfg(feature = "http3")]
impl From<H3SendError> for Error {
    fn from(e: H3SendError) -> Self {
        match e {
            H3SendError::BodyLimit(limit) => limit.error(),
            H3SendError::RequestBody(io) => Error::from_request_body(io),
            H3SendError::NotSent(message) | H3SendError::Rejected(message) => {
                Error::new(Kind::Http3)
                    .with_message(message)
                    .with_source(super::marker::NotProcessed)
            }
            other => Error::new(Kind::Http3).with_message(other.message().into_owned()),
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
