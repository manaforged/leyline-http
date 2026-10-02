use std::fmt;
use std::time::Duration;

use http::StatusCode;
use url::Url;

use super::{CURRENT, ms};
use crate::profile::Browser;
use crate::{Error, HttpVersion};

#[non_exhaustive]
pub struct Summary<'a> {
    pub id: u64,
    pub method: &'a str,
    pub url: Option<&'a Url>,
    pub original_url: &'a str,
    pub redirects: usize,
    pub status: Option<StatusCode>,
    pub version: Option<HttpVersion>,
    pub attempts: u32,
    pub elapsed: Duration,
    pub outcome: Result<(), &'a Error>,
    pub streamed: bool,
    pub tag: Option<&'a str>,
    pub proxy: Option<String>,
    pub browser: Option<Browser>,
}

impl fmt::Debug for Summary<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Summary")
            .field("id", &self.id)
            .field("method", &self.method)
            .field("url", &self.url.map(|u| crate::util::redact(u.as_str())))
            .field("original_url", &crate::util::redact(self.original_url))
            .field("redirects", &self.redirects)
            .field("status", &self.status)
            .field("version", &self.version)
            .field("attempts", &self.attempts)
            .field("elapsed", &self.elapsed)
            .field("outcome", &self.outcome)
            .field("streamed", &self.streamed)
            .field("tag", &self.tag)
            .field("proxy", &self.proxy)
            .field("browser", &self.browser)
            .finish()
    }
}

pub(crate) struct Finish<'a> {
    pub(crate) method: &'a str,
    pub(crate) original_url: &'a str,
    pub(crate) attempts: u32,
    pub(crate) streamed: bool,
}

pub(crate) fn summary(finish: &Finish<'_>, out: Result<&crate::Response, &Error>) {
    let _ = CURRENT.try_with(|ctx| {
        let notes = super::notes::take(&ctx.notes);
        let parsed;
        let (url, redirects, status, version, outcome) = match out {
            Ok(r) => (
                Some(r.url()),
                r.redirect_chain().len(),
                Some(r.status()),
                Some(r.version()),
                Ok(()),
            ),
            Err(e) => {
                parsed = Url::parse(finish.original_url).ok();
                (parsed.as_ref(), 0, None, None, Err(e))
            }
        };
        ctx.hook.summary(&Summary {
            id: ctx.id,
            method: finish.method,
            url,
            original_url: finish.original_url,
            redirects,
            status,
            version,
            attempts: finish.attempts,
            elapsed: ctx.start.elapsed(),
            outcome,
            streamed: finish.streamed,
            tag: notes.tag.as_deref(),
            proxy: notes.proxy,
            browser: notes.browser,
        });
    });
}

pub(super) fn render(ev: &Summary<'_>) {
    let url = crate::util::redact(ev.url.map_or(ev.original_url, Url::as_str));
    let status = ev.status.map(|s| s.as_u16());
    let version = ev.version.map(tracing::field::debug);
    let elapsed_ms = ms(ev.elapsed);
    let tag = ev.tag;
    let proxy = ev.proxy.as_deref();
    let browser = ev.browser.map(tracing::field::debug);
    match ev.outcome {
        Ok(()) => {
            tracing::info!(target: "leyline::trace", id = ev.id, method = ev.method, url = %url, redirects = ev.redirects, status, version, attempts = ev.attempts, elapsed_ms, streamed = ev.streamed, tag, proxy, browser, outcome = "ok", "request")
        }
        Err(e) => {
            tracing::info!(target: "leyline::trace", id = ev.id, method = ev.method, url = %url, redirects = ev.redirects, status, version, attempts = ev.attempts, elapsed_ms, streamed = ev.streamed, tag, proxy, browser, outcome = "error", kind = ?e.kind(), error = %e, "request")
        }
    }
}
