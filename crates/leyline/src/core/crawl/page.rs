use std::collections::HashMap;
use std::fmt;

use http::StatusCode;
use url::Url;

use crate::util::redact;

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
    html: String,
    text: String,
}

impl fmt::Debug for Page {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Page")
            .field("url", &redact(self.url.as_str()))
            .field("depth", &self.depth)
            .field("status", &self.status)
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
        html: String,
        text: String,
    ) -> Self {
        Self {
            url,
            depth,
            status,
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
