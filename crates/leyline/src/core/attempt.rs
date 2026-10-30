use std::fmt;

use http::StatusCode;
use url::Url;

use crate::core::error::{Error, Result};
use crate::core::response::Response;
use crate::util::redact;

/// One sent request or one redirect, as seen by retry and redirect policies.
pub struct Attempt<'a> {
    kind: Kind<'a>,
}

enum Kind<'a> {
    Sent(&'a Result<Response>),
    Redirect {
        status: u16,
        url: &'a Url,
        location: Option<&'a str>,
        previous: &'a [Url],
    },
}

impl<'a> Attempt<'a> {
    pub(crate) fn sent(result: &'a Result<Response>) -> Self {
        Self {
            kind: Kind::Sent(result),
        }
    }

    pub(crate) fn redirect(
        status: u16,
        url: &'a Url,
        location: Option<&'a str>,
        previous: &'a [Url],
    ) -> Self {
        Self {
            kind: Kind::Redirect {
                status,
                url,
                location,
                previous,
            },
        }
    }

    /// Returns the response of a successful send, or `None` for an error or a redirect.
    pub fn response(&self) -> Option<&Response> {
        match self.kind {
            Kind::Sent(result) => result.as_ref().ok(),
            Kind::Redirect { .. } => None,
        }
    }

    /// Returns the error of a failed send, or `None` otherwise.
    pub fn error(&self) -> Option<&Error> {
        match self.kind {
            Kind::Sent(result) => result.as_ref().err(),
            Kind::Redirect { .. } => None,
        }
    }

    /// Returns the response status or the redirect status.
    pub fn status(&self) -> Option<StatusCode> {
        match self.kind {
            Kind::Sent(_) => self.response().map(Response::status),
            Kind::Redirect { status, .. } => StatusCode::from_u16(status).ok(),
        }
    }

    /// Returns the response URL or the URL that sent the redirect.
    pub fn url(&self) -> Option<&Url> {
        match self.kind {
            Kind::Sent(_) => self.response().map(Response::url),
            Kind::Redirect { url, .. } => Some(url),
        }
    }

    /// Returns the redirect's `Location` header value, or `None` for a sent request.
    pub fn location(&self) -> Option<&str> {
        match self.kind {
            Kind::Sent(_) => None,
            Kind::Redirect { location, .. } => location,
        }
    }

    /// Returns the URLs already visited in the redirect chain, empty for a sent request.
    pub fn previous(&self) -> &[Url] {
        match self.kind {
            Kind::Sent(_) => &[],
            Kind::Redirect { previous, .. } => previous,
        }
    }

    /// Returns true when the response matches a bot-block rule.
    pub fn is_block(&self) -> bool {
        self.response().and_then(Response::block).is_some()
    }

    /// Returns true for a 429 or gateway status response or a retryable error, and false for a redirect.
    pub fn is_transient(&self) -> bool {
        match self.kind {
            Kind::Sent(Ok(response)) => {
                let status = response.status();
                status == StatusCode::TOO_MANY_REQUESTS
                    || crate::core::retry::is_gateway_status(status.as_u16())
            }
            Kind::Sent(Err(error)) => error.is_retryable(),
            Kind::Redirect { .. } => false,
        }
    }

    pub(crate) fn is_unsent(&self) -> bool {
        self.error().is_some_and(Error::is_unsent)
    }
}

impl fmt::Debug for Attempt<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let previous: Vec<String> = self
            .previous()
            .iter()
            .map(Url::as_str)
            .map(redact)
            .collect();
        f.debug_struct("Attempt")
            .field("status", &self.status())
            .field("url", &self.url().map(|url| redact(url.as_str())))
            .field("location", &self.location().map(redact))
            .field("previous", &previous)
            .field("error", &self.error().map(Error::kind))
            .finish()
    }
}
