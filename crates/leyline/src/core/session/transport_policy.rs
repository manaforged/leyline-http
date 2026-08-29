use super::{ProtocolPolicy, Session};
use crate::core::body::Body;
use crate::core::error::{Error, Result};

impl Session {
    /// Resolve the proxy URL to use for this request, honoring
    /// (in order of preference): a per-request override, then the
    /// session's default proxy. The result is gated by the no-proxy
    /// matcher, scoped by provenance: an explicit `.no_proxy(...)`
    /// bypasses any proxy; the env-inherited `NO_PROXY` default only
    /// bypasses an env-discovered proxy (see `ProxyConfig::proxy_for`).
    ///
    /// The lifetime is bounded by the shorter of the two inputs so
    /// borrows from either source remain valid.
    fn effective_proxy_for<'a>(
        &'a self,
        url: &url::Url,
        request_proxy: Option<&'a str>,
    ) -> Option<&'a str> {
        self.proxy_config.proxy_for(
            url,
            request_proxy,
            self.proxy.as_deref(),
            self.proxy_from_env,
        )
    }

    /// `true` iff *any* proxy was requested for this call (session
    /// default OR per-request override) — regardless of `NO_PROXY`
    /// filtering. Used by the H3 guard: an explicit caller
    /// `.proxy(...)` must suppress H3 even when `NO_PROXY` would have
    /// bypassed the proxy, because the caller's intent is "route
    /// through this specific egress, not H3."
    #[cfg(feature = "http3")]
    fn proxy_requested(&self, request_proxy: Option<&str>) -> bool {
        request_proxy.is_some() || self.proxy.is_some() || self.proxy_config.first_proxy().is_some()
    }

    // One call site passes the full per-request policy; the arguments are
    // flat wire-request fields, not a configuration object boundary.
    #[expect(
        clippy::too_many_arguments,
        reason = "flat wire-request fields for one internal call site"
    )]
    pub(super) async fn send_with_policy(
        &self,
        method: &str,
        url: &url::Url,
        headers: Vec<crate::h2::connection::HeaderPair>,
        body: Body,
        stream_response: bool,
        request_proxy: Option<&str>,
        header_order: Option<&[String]>,
    ) -> Result<crate::core::transport::TransportResponse> {
        if self.https_only && url.scheme() != "https" {
            return Err(Error::Config(
                "https_only session rejected non-HTTPS URL".into(),
            ));
        }
        let proxy = self.effective_proxy_for(url, request_proxy);
        match self.protocol_policy {
            ProtocolPolicy::Auto => {
                crate::core::transport::send_request_auto(
                    &self.pool,
                    &self.connector,
                    &self.h2_config,
                    method,
                    url,
                    headers,
                    body,
                    proxy,
                    stream_response,
                    header_order,
                )
                .await
            }
            ProtocolPolicy::Http1 => {
                // Boxed cold arm. The H1 transport future is ~10 KB; left
                // inline it would size THIS `match`'s state machine — and
                // therefore every request's per-request future, including the
                // H2 hot path that never selects Http1 — to that 10 KB. Boxing
                // moves the H1 state to the heap, allocated only when an
                // explicit `.http1()` policy actually runs this arm. The H2/Auto
                // hot path pays nothing and the wire bytes are unchanged.
                Box::pin(crate::core::transport::send_request_h1(
                    &self.pool,
                    &self.connector,
                    method,
                    url,
                    headers,
                    body,
                    proxy,
                    stream_response,
                ))
                .await
            }
            ProtocolPolicy::Http2 => {
                crate::core::transport::send_request_h2(
                    &self.pool,
                    &self.connector,
                    &self.h2_config,
                    method,
                    url,
                    headers,
                    body,
                    proxy,
                    stream_response,
                    header_order,
                )
                .await
            }
            #[cfg(feature = "http3")]
            ProtocolPolicy::Http3 => {
                if self.proxy_requested(request_proxy) {
                    return Err(Error::Config(
                        "HTTP/3 over proxies is not implemented; use Auto or Http2".into(),
                    ));
                }
                let h3_config = self.h3_config.as_ref().ok_or_else(|| {
                    Error::Config("this browser profile has no HTTP/3 fingerprint".into())
                })?;
                // Boxed cold arm: the h3 future carries a 64 KB datagram
                // buffer plus the quiche connection, so inlining it sizes
                // every hot-path future by the cold arm. Heavyweight protocol
                // machinery must never size the per-request future.
                Box::pin(crate::core::transport::send_request_h3(
                    &self.pool,
                    h3_config,
                    self.profile,
                    &self.tls_trust,
                    method,
                    url,
                    headers,
                    body,
                    stream_response,
                ))
                .await
            }
        }
    }
}

#[cfg(test)]
mod tests;
