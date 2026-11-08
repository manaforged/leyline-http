use std::fmt;

use http::HeaderValue;

use super::RequestBuilder;
use crate::util::masked;
use crate::util::redact;

impl fmt::Debug for RequestBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestBuilder")
            .field("method", &self.draft.method)
            .field(
                "url",
                &redact(self.draft.url.as_deref().map_or("", url::Url::as_str)),
            )
            .field("query", &masked_query(&self.draft.query_params))
            .field(
                "headers",
                &masked(
                    self.draft
                        .headers
                        .iter()
                        .map(|(k, v)| (k.as_str(), shown(v))),
                ),
            )
            .field("body", &self.draft.body)
            .field("preset", &self.draft.carry.preset)
            .field("timeouts", &self.draft.carry.timeouts)
            .field("retry_policy", &self.draft.carry.retry_policy)
            .field("proxy", &self.draft.carry.proxy)
            .field("redirect", &self.draft.carry.redirect)
            .field(
                "digest_auth",
                &self.draft.carry.digest_auth.as_ref().map(|_| "***"),
            )
            .field("compress", &self.draft.compress)
            .field("stream_response", &self.draft.carry.stream_response)
            .field("header_order", &self.draft.carry.header_order)
            .field("error", &self.draft.builder_error)
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
