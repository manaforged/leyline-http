use std::collections::HashMap;
use std::fmt;

use http::StatusCode;
use url::Url;

use crate::html;

#[derive(Debug, Clone)]
pub struct CrawlPage {
    url: Url,
    depth: usize,
    status: StatusCode,
    html: String,
    text: String,
}

impl CrawlPage {
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

    #[must_use]
    pub fn url(&self) -> &Url {
        &self.url
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.depth
    }

    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    #[must_use]
    pub fn html(&self) -> &str {
        &self.html
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn meta(&self, name: &str) -> Option<String> {
        html::meta(&self.html, name)
    }

    #[must_use]
    pub fn into_html(self) -> String {
        self.html
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SkipReason {
    Robots,
    Signal,
    Status,
    NotHtml,
    Blocked,
    NoIndex,
    NoAi,
    Duplicate,
    Redirects,
    Scope,
}

impl SkipReason {
    const ALL: [SkipReason; 10] = [
        SkipReason::Robots,
        SkipReason::Signal,
        SkipReason::Status,
        SkipReason::NotHtml,
        SkipReason::Blocked,
        SkipReason::NoIndex,
        SkipReason::NoAi,
        SkipReason::Duplicate,
        SkipReason::Redirects,
        SkipReason::Scope,
    ];

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

#[derive(Debug, Clone, Default)]
pub struct CrawlStats {
    requests: u64,
    pages: u64,
    errors: u64,
    skipped: HashMap<SkipReason, u64>,
}

impl CrawlStats {
    #[must_use]
    pub fn requests(&self) -> u64 {
        self.requests
    }

    #[must_use]
    pub fn pages(&self) -> u64 {
        self.pages
    }

    #[must_use]
    pub fn errors(&self) -> u64 {
        self.errors
    }

    #[must_use]
    pub fn skipped(&self, reason: SkipReason) -> u64 {
        self.skipped.get(&reason).copied().unwrap_or(0)
    }

    pub(super) fn request(&mut self) {
        self.requests += 1;
    }

    pub(super) fn page(&mut self) {
        self.pages += 1;
    }

    pub(super) fn error(&mut self) {
        self.errors += 1;
    }

    pub(super) fn skip(&mut self, reason: SkipReason) {
        *self.skipped.entry(reason).or_default() += 1;
    }
}

impl fmt::Display for CrawlStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "requests={} pages={} errors={}",
            self.requests, self.pages, self.errors
        )?;
        for reason in SkipReason::ALL {
            write!(f, " skipped_{}={}", reason.as_str(), self.skipped(reason))?;
        }
        Ok(())
    }
}
