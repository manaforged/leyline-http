use http::HeaderMap;
use http::header::{
    CONNECTION, CONTENT_ENCODING, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, TRAILER,
};

use super::Response;
use crate::core::decompress::{Decoder, strip_decoded};

impl Response {
    /// Returns the headers to forward to a client, without hop-by-hop headers and encoding headers for a decoded body.
    pub fn relay_headers(&self) -> HeaderMap {
        let mut out = relay_headers(self.parts.headers.map());
        if self.decodes_body() && out.contains_key(CONTENT_ENCODING) {
            strip_decoded(&mut out);
        }
        out
    }

    pub(crate) fn decodes_body(&self) -> bool {
        Decoder::new(&self.content_coding(), &self.parts.compression)
            .is_ok_and(|decoder| decoder.is_some())
    }
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
    for name in crate::wire::HOP_BY_HOP {
        out.remove(name);
    }
    for name in [PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, TRAILER] {
        out.remove(name);
    }
    for name in &listed {
        out.remove(name.as_str());
    }
    out
}
