use std::fmt;

use http::HeaderValue;

use super::RequestBuilder;
use crate::trace::masked;
use crate::util::redact;

impl fmt::Debug for RequestBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestBuilder")
            .field("method", &self.method)
            .field("url", &redact(&self.url))
            .field("query", &masked_query(&self.query_params))
            .field(
                "headers",
                &masked(self.headers.iter().map(|(k, v)| (k.as_str(), shown(v)))),
            )
            .field("body", &self.body)
            .field("preset", &self.preset)
            .field("timeouts", &self.timeouts)
            .field("retry_policy", &self.retry_policy)
            .field("proxy", &self.proxy)
            .field("redirect", &self.redirect)
            .field("digest_auth", &self.digest_auth.as_ref().map(|_| "***"))
            .field("compress", &self.compress)
            .field("stream_response", &self.stream_response)
            .field("header_order", &self.header_order)
            .field("error", &self.builder_error)
            .finish_non_exhaustive()
    }
}

fn shown(value: &HeaderValue) -> &[u8] {
    if value.is_sensitive() {
        b"***"
    } else {
        value.as_bytes()
    }
}

fn masked_query(params: &[(String, String)]) -> Vec<(&str, &str)> {
    params
        .iter()
        .map(|(name, _)| (name.as_str(), "***"))
        .collect()
}
