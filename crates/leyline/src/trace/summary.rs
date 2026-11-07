use url::Url;

use super::Event;
use crate::error::Error;
use crate::wire::ResponseTiming;

pub(crate) struct Finish<'a> {
    pub(crate) method: &'a str,
    pub(crate) original_url: &'a str,
    pub(crate) attempts: u32,
    pub(crate) streamed: bool,
}

pub(crate) struct Answer<'a> {
    pub(crate) url: &'a Url,
    pub(crate) redirects: usize,
    pub(crate) status: http::StatusCode,
    pub(crate) version: http::Version,
}

pub(crate) fn summary(finish: &Finish<'_>, out: Result<Answer<'_>, &Error>) {
    super::with(|ctx| {
        let notes = super::notes::take(&ctx.notes);
        let parsed;
        let (url, redirects, status, version, outcome) = match out {
            Ok(answer) => (
                Some(answer.url),
                answer.redirects,
                Some(answer.status),
                Some(answer.version),
                Ok(()),
            ),
            Err(e) => {
                parsed = Url::parse(finish.original_url).ok();
                (parsed.as_ref(), 0, None, None, Err(e))
            }
        };
        ctx.hook.event(&Event::Summary {
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
            label: notes.label.as_deref(),
            proxy: notes.proxy,
            browser: notes.browser,
        });
    });
}

pub(super) fn render(event: &Event<'_>) {
    let Event::Summary {
        id,
        method,
        url,
        original_url,
        redirects,
        status,
        version,
        attempts,
        elapsed,
        outcome,
        streamed,
        label,
        proxy,
        browser,
    } = event
    else {
        return;
    };
    let url = crate::util::redact(url.map_or(*original_url, Url::as_str));
    let status = status.map(|s| s.as_u16());
    let version = version.map(tracing::field::debug);
    let elapsed_ms = ResponseTiming::span(*elapsed);
    let label = *label;
    let proxy = proxy.as_deref();
    let browser = browser.map(tracing::field::debug);
    match outcome {
        Ok(()) => {
            tracing::info!(target: "leyline::trace", id = *id, method = *method, url = %url, redirects = *redirects, status, version, attempts = *attempts, elapsed_ms, streamed = *streamed, label, proxy, browser, outcome = "ok", "request")
        }
        Err(e) => {
            tracing::info!(target: "leyline::trace", id = *id, method = *method, url = %url, redirects = *redirects, status, version, attempts = *attempts, elapsed_ms, streamed = *streamed, label, proxy, browser, outcome = "error", kind = ?e.kind(), error = %e, "request")
        }
    }
}
