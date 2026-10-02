use std::fmt;

use super::SessionBuilder;
use crate::trace::masked;

impl fmt::Debug for SessionBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let profile = self.profile.as_ref().map(|p| p.meta.name.as_str());
        f.debug_struct("SessionBuilder")
            .field("browser", &self.browser)
            .field("profile", &profile)
            .field("platform", &self.platform)
            .field("brand", &self.brand)
            .field("http_identity", &self.http_identity)
            .field("protocol_policy", &self.protocol_policy)
            .field("proxy", &self.proxy_config)
            .field("dns", &self.dns_config)
            .field("timeouts", &self.timeouts)
            .field("pool", &self.pool_config)
            .field("socket", &self.socket_config)
            .field("redirect", &self.redirect_policy)
            .field("compression", &self.compression)
            .field("websocket", &self.websocket_config)
            .field("retry", &self.default_retry)
            .field("https_only", &self.https_only)
            .field("audit", &self.audit)
            .field("cookie_jar", &self.cookie_jar)
            .field("tcp_profile", &self.tcp_profile)
            .field("tls_trust", &self.tls_trust)
            .field(
                "default_headers",
                &masked(
                    self.default_headers
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_bytes())),
                ),
            )
            .field("bearer", &self.bearer.is_some())
            .field(
                "base_url",
                &self
                    .base_url
                    .as_ref()
                    .map(|url| crate::util::redact(url.as_str())),
            )
            .field("languages", &self.languages)
            .field("host_limits", &self.host_limits)
            .field("proxy_pool", &self.proxy_pool.is_some())
            .field("trace", &self.trace.is_some())
            .field("config_error", &self.config_error)
            .finish_non_exhaustive()
    }
}
