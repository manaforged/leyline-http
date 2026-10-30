use http::HeaderMap;
use http::header::{
    CONNECTION, CONTENT_ENCODING, CONTENT_LENGTH, HeaderName, PROXY_AUTHENTICATE,
    PROXY_AUTHORIZATION, TE, TRAILER, TRANSFER_ENCODING, UPGRADE,
};

use super::Response;
use crate::core::session::decompress::Decoder;

const KEEP_ALIVE: &str = "keep-alive";
const PROXY_CONNECTION: &str = "proxy-connection";

impl Response {
    /// Returns the headers to forward to a client, without hop-by-hop headers and encoding headers for a decoded body.
    pub fn relay_headers(&self) -> HeaderMap {
        let mut out = relay_headers(&self.headers);
        if self.decodes_body() && out.contains_key(CONTENT_ENCODING) {
            out.remove(CONTENT_ENCODING);
            out.remove(CONTENT_LENGTH);
        }
        out
    }

    pub(crate) fn decodes_body(&self) -> bool {
        let encoding = self.content_coding();
        Decoder::new(encoding.as_deref(), &self.compression).is_ok_and(|decoder| decoder.is_some())
    }
}

fn hop_by_hop() -> [HeaderName; 9] {
    [
        CONNECTION,
        HeaderName::from_static(KEEP_ALIVE),
        HeaderName::from_static(PROXY_CONNECTION),
        PROXY_AUTHENTICATE,
        PROXY_AUTHORIZATION,
        TE,
        TRAILER,
        TRANSFER_ENCODING,
        UPGRADE,
    ]
}

/// Returns a copy of `headers` without hop-by-hop headers.
pub fn relay_headers(headers: &HeaderMap) -> HeaderMap {
    let mut out = headers.clone();
    let listed: Vec<String> = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| !token.is_empty())
        .collect();
    for name in hop_by_hop() {
        out.remove(name);
    }
    for name in &listed {
        out.remove(name.as_str());
    }
    out
}
