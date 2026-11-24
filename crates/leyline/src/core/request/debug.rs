use std::fmt;

use http::HeaderValue;

use super::RequestBuilder;
use crate::util::masked;
use crate::util::redact;

impl fmt::Debug for RequestBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let overrides = self.draft.carry.overrides();
        let build = self.draft.build_extras();
        let query = build.map_or(&[][..], |b| b.query_params.as_slice());
        f.debug_struct("RequestBuilder")
            .field("method", &self.draft.method)
            .field(
                "url",
                &redact(self.draft.url.as_deref().map_or("", url::Url::as_str)),
            )
            .field("query", &masked_query(query))
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
            .field("timeouts", &overrides.and_then(|o| o.timeouts.as_ref()))
            .field("retry_policy", self.draft.carry.retry_policy(&self.session))
            .field("proxy", &overrides.and_then(|o| o.proxy.as_ref()))
            .field("protocol", &self.draft.carry.protocol)
            .field("redirect", &overrides.and_then(|o| o.redirect.as_ref()))
            .field(
                "digest_auth",
                &overrides.and_then(|o| o.digest_auth.as_ref().map(|_| "***")),
            )
            .field("compress", &build.and_then(|b| b.compress))
            .field("stream_response", &self.draft.carry.stream_response)
            .field(
                "header_order",
                &overrides.and_then(|o| o.header_order.as_ref()),
            )
            .field("error", &build.and_then(|b| b.builder_error.as_ref()))
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
