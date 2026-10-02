use http::HeaderMap;
use http::header::{
    CONNECTION, CONTENT_ENCODING, CONTENT_LENGTH, HeaderName, PROXY_AUTHENTICATE,
    PROXY_AUTHORIZATION, TE, TRAILER, TRANSFER_ENCODING, UPGRADE,
};

use super::Response;
use crate::core::session::decompress::{Decoder, content_codings};

const KEEP_ALIVE: &str = "keep-alive";
const PROXY_CONNECTION: &str = "proxy-connection";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RelayBody {
    AsReceived,
    Decoded,
}

impl Response {
    pub fn relay_headers(&self, body: RelayBody) -> HeaderMap {
        let body = match body {
            RelayBody::Decoded if !self.decodes_body() => RelayBody::AsReceived,
            other => other,
        };
        relay_headers(&self.headers, body)
    }

    fn decodes_body(&self) -> bool {
        let encoding = content_codings(self.headers.get_all(CONTENT_ENCODING));
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

pub fn relay_headers(headers: &HeaderMap, body: RelayBody) -> HeaderMap {
    let mut out = headers.clone();
    if body == RelayBody::Decoded && out.contains_key(CONTENT_ENCODING) {
        out.remove(CONTENT_ENCODING);
        out.remove(CONTENT_LENGTH);
    }
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
