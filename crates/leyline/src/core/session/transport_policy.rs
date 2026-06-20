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
    /// filtering. Used by the H3 / Race guards: an explicit caller
    /// `.proxy(...)` must suppress H3 even when `NO_PROXY` would have
    /// bypassed the proxy, because the caller's intent is "route
    /// through this specific egress, not H3."
    #[cfg(feature = "http3")]
    fn proxy_requested(&self, request_proxy: Option<&str>) -> bool {
        request_proxy.is_some() || self.proxy.is_some() || self.proxy_config.first_proxy().is_some()
    }

    pub(super) async fn send_with_policy(
        &self,
        method: &str,
        url: &url::Url,
        headers: Vec<(String, String)>,
        body: Body,
        stream_response: bool,
        request_proxy: Option<&str>,
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
                // Boxed cold arm (defense-in-depth). The companion change in
                // this same commit (`quic::connection`) already moves the H3
                // future's 64 KB datagram buffer + quiche Connection to the
                // heap, so the H3 future is now ~2.5 KB — not the ~82 KB it was
                // when, inlined, it became >97% of every request's allocation.
                // We still box the arm on principle: heavyweight cold protocol
                // machinery must never size the hot per-request future, so
                // future growth in the H3 path cannot silently re-inflate the
                // H2/Auto hot path (this mirrors wreq's boxed-protocol
                // dispatch). The ~10 KB H1 arm above is the larger remaining
                // payoff of arm-boxing today.
                Box::pin(crate::core::transport::send_request_h3(
                    h3_config,
                    self.profile,
                    method,
                    url,
                    headers,
                    body,
                    stream_response,
                ))
                .await
            }
            #[cfg(feature = "http3")]
            ProtocolPolicy::Race => {
                // Race doesn't interact well with streaming bodies — we
                // can only try H3 first if we have a buffered body to
                // keep for the fallback. Streaming bodies run straight
                // through the Auto path.
                if body.is_stream() || stream_response {
                    return crate::core::transport::send_request_auto(
                        &self.pool,
                        &self.connector,
                        &self.h2_config,
                        method,
                        url,
                        headers,
                        body,
                        proxy,
                        stream_response,
                    )
                    .await;
                }
                if !self.proxy_requested(request_proxy) && url.scheme() == "https" {
                    // Race the H3 attempt only if this profile has an H3
                    // fingerprint; otherwise fall straight through to H2.
                    if let Some(h3_config) = self.h3_config.as_ref() {
                        // We have a buffered body — clone for the retry.
                        let retained = match &body {
                            Body::Empty => Body::Empty,
                            Body::Bytes(b) => Body::Bytes(b.clone()),
                            Body::Stream { .. } => unreachable!(),
                        };
                        // Boxed cold arm (defense-in-depth, as in the Http3
                        // arm). The common outcome here is the H2 fallback
                        // below, so the H3 attempt's state stays off the inline
                        // Race future. The QUIC future is already ~2.5 KB after
                        // this commit's `quic::connection` change; boxing keeps
                        // it that way on principle.
                        if let Ok(resp) = Box::pin(crate::core::transport::send_request_h3(
                            h3_config,
                            self.profile,
                            method,
                            url,
                            headers.clone(),
                            retained,
                            stream_response,
                        ))
                        .await
                        {
                            return Ok(resp);
                        }
                    }
                }
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
                )
                .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// `with_proxy` must actually override a proxy set at build time.
    /// `proxy_for` returns the FIRST matching rule, so appending an
    /// all-scheme rule would let the original build-time proxy keep
    /// winning and silently no-op the rotation.
    #[test]
    fn with_proxy_overrides_build_time_proxy() {
        let session = crate::Session::builder()
            .proxy("http://first:1")
            .build()
            .expect("bare session builds");
        let rotated = session.with_proxy("http://second:2");

        let url = url::Url::parse("https://example.test/").unwrap();
        assert_eq!(
            rotated.effective_proxy_for(&url, None),
            Some("http://second:2"),
            "with_proxy must win over the build-time proxy"
        );
    }
}
