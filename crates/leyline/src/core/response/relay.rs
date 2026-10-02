use http::HeaderMap;
use http::header::{CONNECTION, CONTENT_ENCODING, CONTENT_LENGTH};

use super::Response;

const HOP_BY_HOP: [&str; 7] = [
    "connection",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayBody {
    AsReceived,
    Decoded,
}

impl Response {
    pub fn relay_headers(&self, body: RelayBody) -> HeaderMap {
        relay_headers(&self.headers, body)
    }
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
    for name in HOP_BY_HOP
        .iter()
        .copied()
        .chain(listed.iter().map(String::as_str))
    {
        out.remove(name);
    }
    out
}
