use http::{HeaderMap, StatusCode};
use url::Url;

use super::SkipReason;
use crate::core::config::RedirectPolicy;
use crate::core::error::Result;
use crate::core::session::Session;

const HTML_TYPES: [&str; 2] = ["text/html", "application/xhtml+xml"];
const HTML_SIGNATURES: [&str; 17] = [
    "<!DOCTYPE HTML",
    "<HTML",
    "<HEAD",
    "<SCRIPT",
    "<IFRAME",
    "<H1",
    "<DIV",
    "<FONT",
    "<TABLE",
    "<A",
    "<STYLE",
    "<TITLE",
    "<B",
    "<BODY",
    "<BR",
    "<P",
    "<!--",
];
const SNIFF_WHITESPACE: [char; 5] = ['\t', '\n', '\x0c', '\r', ' '];
const LOCATION: &str = "location";
const CONTENT_TYPE: &str = "content-type";

pub(super) enum Fetched {
    Redirect(Url),
    Skip(SkipReason),
    Page(Box<Loaded>),
}

pub(super) struct Loaded {
    pub(super) url: Url,
    pub(super) status: StatusCode,
    pub(super) headers: HeaderMap,
    pub(super) html: String,
}

fn is_html(content_type: &str) -> bool {
    let media = content_type.split(';').next().unwrap_or_default().trim();
    HTML_TYPES
        .iter()
        .any(|kind| kind.eq_ignore_ascii_case(media))
}

fn sniffs_html(body: &str) -> bool {
    let start = body.trim_start_matches(SNIFF_WHITESPACE).as_bytes();
    HTML_SIGNATURES.iter().any(|signature| {
        let signature = signature.as_bytes();
        start.len() > signature.len()
            && start[..signature.len()].eq_ignore_ascii_case(signature)
            && matches!(start[signature.len()], b' ' | b'>')
    })
}

pub(super) async fn fetch(session: Session, url: Url) -> Result<Fetched> {
    let response = session
        .get(url.as_str())
        .redirect(RedirectPolicy::none())
        .stream()
        .send()
        .await?;
    let status = response.status();
    if status.is_redirection() {
        let target = response.header(LOCATION).and_then(|to| url.join(to).ok());
        return Ok(target.map_or(Fetched::Skip(SkipReason::Status), Fetched::Redirect));
    }
    if response.block().is_some() {
        return Ok(Fetched::Skip(SkipReason::Blocked));
    }
    if !status.is_success() {
        return Ok(Fetched::Skip(SkipReason::Status));
    }
    let declared = response.header(CONTENT_TYPE).map(is_html);
    if declared == Some(false) {
        return Ok(Fetched::Skip(SkipReason::NotHtml));
    }
    let url = response.url().clone();
    let headers = response.headers().clone();
    let html = response.text().await?;
    if declared.is_none() && !sniffs_html(&html) {
        return Ok(Fetched::Skip(SkipReason::NotHtml));
    }
    Ok(Fetched::Page(Box::new(Loaded {
        url,
        status,
        headers,
        html,
    })))
}
