use crate::core::error::{Error, Result};
use crate::h2::connection::PseudoHeaders;
use crate::header_str::HeaderStr;
use crate::util::request_target;

const STANDARD_METHODS: [&str; 9] = [
    "GET", "HEAD", "POST", "PUT", "DELETE", "OPTIONS", "PATCH", "TRACE", "CONNECT",
];

fn method_header(method: &str) -> HeaderStr {
    STANDARD_METHODS
        .iter()
        .copied()
        .find(|known| *known == method)
        .map_or_else(|| HeaderStr::from(method), HeaderStr::from_static)
}

pub(super) fn request_pseudo(method: &str, url: &url::Url) -> Result<PseudoHeaders> {
    if url.scheme() != "https" {
        return Err(Error::config("HTTP/2 requires an https:// URL"));
    }
    if !url.has_host() {
        return Err(Error::config("no host in URL"));
    }
    Ok(PseudoHeaders {
        method: method_header(method),
        scheme: HeaderStr::from_static("https"),
        authority: HeaderStr::from(&url[url::Position::BeforeHost..url::Position::AfterPort]),
        path: HeaderStr::from(request_target(url)),
        protocol: None,
    })
}
