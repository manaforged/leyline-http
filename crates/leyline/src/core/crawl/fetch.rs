use http::header::LOCATION;
use http::{HeaderMap, HeaderName, StatusCode};
use url::Url;

use super::SkipReason;
use crate::core::config::RedirectPolicy;
use crate::core::error::Result;
use crate::core::request::RequestBuilder;
use crate::core::retry::RetryPolicy;
use crate::core::session::Session;

const HTML_TYPES: [&str; 2] = ["text/html", "application/xhtml+xml"];
const HTML_SIGNATURES: [&[u8]; 17] = [
    b"<!DOCTYPE HTML",
    b"<HTML",
    b"<HEAD",
    b"<SCRIPT",
    b"<IFRAME",
    b"<H1",
    b"<DIV",
    b"<FONT",
    b"<TABLE",
    b"<A",
    b"<STYLE",
    b"<TITLE",
    b"<B",
    b"<BODY",
    b"<BR",
    b"<P",
    b"<!--",
];
const SNIFF_WHITESPACE: [u8; 5] = [b'\t', b'\n', 0x0c, b'\r', b' '];
const TAG_ENDS: [u8; 6] = [b'\t', b'\n', 0x0c, b'\r', b' ', b'>'];
const SNIFF_BYTES: usize = 512;
const MAX_UNTYPED_BYTES: usize = 8 * 1024 * 1024;
pub(super) const X_ROBOTS_TAG: HeaderName = HeaderName::from_static("x-robots-tag");
pub(super) const CONTENT_USAGE: HeaderName = HeaderName::from_static("content-usage");
const KEPT_HEADERS: [HeaderName; 2] = [X_ROBOTS_TAG, CONTENT_USAGE];

pub(super) enum Fetched {
    Redirect { to: Url },
    Skip(SkipReason),
    Page(Box<Loaded>),
}

pub(super) struct Loaded {
    pub(super) url: Url,
    pub(super) status: StatusCode,
    pub(super) headers: HeaderMap,
    pub(super) html: String,
}

fn is_html(media: &str) -> bool {
    HTML_TYPES
        .iter()
        .any(|kind| kind.eq_ignore_ascii_case(media))
}

fn sniffs_html(body: &[u8]) -> bool {
    let skip = body
        .iter()
        .take_while(|byte| SNIFF_WHITESPACE.contains(byte))
        .count();
    let start = &body[skip..];
    HTML_SIGNATURES.iter().any(|signature| {
        start.len() > signature.len()
            && start[..signature.len()].eq_ignore_ascii_case(signature)
            && TAG_ENDS.contains(&start[signature.len()])
    })
}

pub(super) fn request(
    session: &Session,
    url: &Url,
    retry: RetryPolicy,
    trusted: Option<Url>,
) -> RequestBuilder {
    let mut builder = session
        .get(url.as_str())
        .redirect(RedirectPolicy::none())
        .retry(retry);
    builder.trusted_origin = trusted;
    builder
}

fn kept_headers(all: &HeaderMap) -> HeaderMap {
    let mut kept = HeaderMap::new();
    for name in KEPT_HEADERS {
        for value in all.get_all(&name) {
            kept.append(name.clone(), value.clone());
        }
    }
    kept
}

pub(super) async fn fetch(
    session: Session,
    url: Url,
    retry: RetryPolicy,
    trusted: Option<Url>,
) -> Result<Fetched> {
    let response = request(&session, &url, retry, trusted)
        .stream()
        .send()
        .await?;
    let status = response.status();
    if status.is_redirection() {
        let target = response
            .header(LOCATION.as_str())
            .and_then(|to| url.join(to).ok());
        return Ok(match target {
            Some(to) => Fetched::Redirect { to },
            None => Fetched::Skip(SkipReason::Status),
        });
    }
    if response.block().is_some() {
        return Ok(Fetched::Skip(SkipReason::Blocked));
    }
    if !status.is_success() {
        return Ok(Fetched::Skip(SkipReason::Status));
    }
    let declared = response.media_type().map(is_html);
    if declared == Some(false) {
        return Ok(Fetched::Skip(SkipReason::NotHtml));
    }
    let url = response.url().clone();
    let headers = kept_headers(response.headers());
    let html = match declared {
        Some(_) => response.text().await?,
        None => {
            let body = response
                .read_until(MAX_UNTYPED_BYTES, |body, _| {
                    body.len() >= SNIFF_BYTES && !sniffs_html(body)
                })
                .await?;
            if !sniffs_html(&body) {
                return Ok(Fetched::Skip(SkipReason::NotHtml));
            }
            String::from_utf8_lossy(&body).into_owned()
        }
    };
    Ok(Fetched::Page(Box::new(Loaded {
        url,
        status,
        headers,
        html,
    })))
}
