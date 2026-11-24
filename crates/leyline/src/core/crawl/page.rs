use std::collections::HashMap;
use std::fmt;

use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use url::Url;

use crate::util::{header_str, masked, redact};

/// A fetched HTML page that a crawl yields.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Page {
    url: Url,
    depth: usize,
    #[serde(
        serialize_with = "status_serialize",
        deserialize_with = "status_deserialize"
    )]
    status: StatusCode,
    #[serde(
        default,
        with = "header_map_serde",
        skip_serializing_if = "HeaderMap::is_empty"
    )]
    headers: HeaderMap,
    html: String,
    text: String,
}

impl fmt::Debug for Page {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Page")
            .field("url", &redact(self.url.as_str()))
            .field("depth", &self.depth)
            .field("status", &self.status)
            .field(
                "headers",
                &masked(self.headers.iter().map(|(k, v)| (k.as_str(), v.as_bytes()))),
            )
            .field("html_bytes", &self.html.len())
            .field("text_bytes", &self.text.len())
            .finish()
    }
}

impl Page {
    pub(super) fn new(
        url: Url,
        depth: usize,
        status: StatusCode,
        headers: HeaderMap,
        html: String,
        text: String,
    ) -> Self {
        Self {
            url,
            depth,
            status,
            headers,
            html,
            text,
        }
    }

    /// Returns the page's final URL.
    #[must_use]
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Returns the page's crawl depth, 0 for the seed.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// Returns the HTTP status of the page response.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns the response headers kept on the page, excluding sensitive credential headers such as `Set-Cookie`.
    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Returns the named page response header as text, or `None` when it is absent or not visible ASCII.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        header_str(&self.headers, name)
    }

    /// Returns the page's HTML body.
    #[must_use]
    pub fn html(&self) -> &str {
        &self.html
    }

    /// Returns the page's extracted text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Consumes the page and returns its HTML body.
    #[must_use]
    pub fn into_html(self) -> String {
        self.html
    }
}

/// Why a crawl skipped a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum SkipReason {
    /// Robots.txt disallows the URL.
    Robots,
    /// A robots rule or response header refuses the crawl's purpose.
    Signal,
    /// The response status was not a success, or a redirect had no target.
    Status,
    /// The response was not HTML.
    NotHtml,
    /// The response was detected as a block page.
    Blocked,
    /// The page carries a noindex directive.
    NoIndex,
    /// The page refuses AI use and the crawl's purpose uses AI.
    NoAi,
    /// The URL or page was already seen.
    Duplicate,
    /// The URL exceeded the redirect hop limit.
    Redirects,
    /// The URL is outside the crawl scope.
    Scope,
    /// The crawl's `filter` rejected the URL.
    Filter,
}

impl SkipReason {
    /// Returns the reason as a snake_case label such as `not_html`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            SkipReason::Robots => "robots",
            SkipReason::Signal => "signal",
            SkipReason::Status => "status",
            SkipReason::NotHtml => "not_html",
            SkipReason::Blocked => "blocked",
            SkipReason::NoIndex => "noindex",
            SkipReason::NoAi => "noai",
            SkipReason::Duplicate => "duplicate",
            SkipReason::Redirects => "redirects",
            SkipReason::Scope => "scope",
            SkipReason::Filter => "filter",
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct CrawlStats {
    requests: u64,
    pages: u64,
    errors: u64,
    skipped: HashMap<SkipReason, u64>,
}

fn status_serialize<S: serde::Serializer>(
    status: &StatusCode,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_u16(status.as_u16())
}

fn status_deserialize<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<StatusCode, D::Error> {
    let code = <u16 as serde::Deserialize>::deserialize(deserializer)?;
    StatusCode::from_u16(code).map_err(serde::de::Error::custom)
}

mod header_map_serde {
    use super::{HeaderMap, HeaderName, HeaderValue};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize)]
    #[serde(untagged)]
    enum WireValue<'a> {
        Text(&'a str),
        Bytes(&'a [u8]),
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OwnedValue {
        Text(String),
        Bytes(Vec<u8>),
    }

    pub(super) fn serialize<S: Serializer>(
        headers: &HeaderMap,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let pairs: Vec<(&str, WireValue<'_>)> = headers
            .iter()
            .map(|(name, value)| {
                let wire = match value.to_str() {
                    Ok(text) => WireValue::Text(text),
                    Err(_) => WireValue::Bytes(value.as_bytes()),
                };
                (name.as_str(), wire)
            })
            .collect();
        pairs.serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<HeaderMap, D::Error> {
        let pairs = Vec::<(String, OwnedValue)>::deserialize(deserializer)?;
        let mut map = HeaderMap::with_capacity(pairs.len());
        for (name, value) in pairs {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(serde::de::Error::custom)?;
            let bytes = match &value {
                OwnedValue::Text(text) => text.as_bytes(),
                OwnedValue::Bytes(raw) => raw.as_slice(),
            };
            let value = HeaderValue::from_bytes(bytes).map_err(serde::de::Error::custom)?;
            map.append(name, value);
        }
        Ok(map)
    }
}

impl CrawlStats {
    pub(super) fn requests(&self) -> u64 {
        self.requests
    }

    pub(super) fn pages(&self) -> u64 {
        self.pages
    }

    pub(super) fn errors(&self) -> u64 {
        self.errors
    }

    pub(super) fn skipped(&self, reason: SkipReason) -> u64 {
        self.skipped.get(&reason).copied().unwrap_or(0)
    }

    pub(super) fn request(&mut self) {
        self.requests = self.requests.saturating_add(1);
    }

    pub(super) fn page(&mut self) {
        self.pages = self.pages.saturating_add(1);
    }

    pub(super) fn error(&mut self) {
        self.errors = self.errors.saturating_add(1);
    }

    pub(super) fn skip(&mut self, reason: SkipReason) {
        let count = self.skipped.entry(reason).or_default();
        *count = count.saturating_add(1);
    }
}
