use std::fmt;

use http::HeaderValue;

use super::RequestBuilder;
use crate::trace::masked;
use crate::util::without_userinfo;

impl fmt::Debug for RequestBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestBuilder")
            .field("method", &self.method)
            .field("url", &shown_url(&self.url))
            .field("query", &self.query_params)
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

fn shown_url(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(parsed) => without_userinfo(parsed).to_string(),
        Err(_) => raw.to_owned(),
    }
}
