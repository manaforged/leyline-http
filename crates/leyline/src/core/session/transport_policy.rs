use super::{ProtocolPolicy, Session};
use crate::core::body::Body;
use crate::core::error::{Error, Result};
#[cfg(feature = "http3")]
use crate::core::transport::send_request_h3;
use crate::core::transport::{
    TransportResponse, send_request_auto, send_request_h1, send_request_h2,
};
use crate::h2::connection::HeaderPair;
#[cfg(feature = "http3")]
use crate::pool::checkout_h3_handle;
use crate::pool::checkout_handle;
#[cfg(feature = "http3")]
use crate::quic::H3Config;

impl Session {
    /// Resolve the proxy URL to use for this request, honoring (in order of preference): a per-request override, then the session's default proxy.
    fn effective_proxy_for<'a>(
        &'a self,
        url: &url::Url,
        request_proxy: Option<&'a str>,
    ) -> Option<&'a str> {
        self.inner.proxy_config.proxy_for(
            url,
            request_proxy,
            self.inner.proxy.as_deref(),
            self.inner.proxy_from_env,
        )
    }

    /// `true` iff *any* proxy was requested for this call (session default OR per-request override) — regardless of `NO_PROXY` filtering.
    #[cfg(feature = "http3")]
    fn proxy_requested(&self, request_proxy: Option<&str>) -> bool {
        request_proxy.is_some()
            || self.inner.proxy.is_some()
            || self.inner.proxy_config.first_proxy().is_some()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "flat wire-request fields for one internal call site"
    )]
    pub(super) async fn send_with_policy(
        &self,
        method: &str,
        url: &url::Url,
        headers: Vec<HeaderPair>,
        body: Body,
        stream_response: bool,
        request_proxy: Option<&str>,
        header_order: Option<&[String]>,
    ) -> Result<TransportResponse> {
        if self.inner.https_only && url.scheme() != "https" {
            return Err(Error::Config(
                "https_only session rejected non-HTTPS URL".into(),
            ));
        }
        let proxy = self.effective_proxy_for(url, request_proxy);
        match self.inner.protocol_policy {
            ProtocolPolicy::Auto => {
                send_request_auto(
                    &self.inner.pool,
                    &self.inner.connector,
                    &self.inner.h2_config,
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
                Box::pin(send_request_h1(
                    &self.inner.pool,
                    &self.inner.connector,
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
                send_request_h2(
                    &self.inner.pool,
                    &self.inner.connector,
                    &self.inner.h2_config,
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
                let h3_config = self.inner.h3_config.as_ref().ok_or_else(|| {
                    Error::Config("this browser profile has no HTTP/3 fingerprint".into())
                })?;
                Box::pin(send_request_h3(
                    &self.inner.pool,
                    h3_config,
                    self.inner.profile,
                    &self.inner.tls_trust,
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
                let raceable = !body.is_stream()
                    && !stream_response
                    && !self.proxy_requested(request_proxy)
                    && url.scheme() == "https";
                match (raceable, self.inner.h3_config.as_ref()) {
                    (true, Some(h3_config)) => {
                        self.send_raced(h3_config, method, url, headers, body, proxy, header_order)
                            .await
                    }
                    _ => {
                        send_request_auto(
                            &self.inner.pool,
                            &self.inner.connector,
                            &self.inner.h2_config,
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
                }
            }
        }
    }

    /// Race QUIC against TCP+TLS.
    #[cfg(feature = "http3")]
    #[expect(
        clippy::too_many_arguments,
        reason = "flat wire-request fields for one internal call site"
    )]
    async fn send_raced(
        &self,
        h3_config: &H3Config,
        method: &str,
        url: &url::Url,
        headers: Vec<HeaderPair>,
        body: Body,
        proxy: Option<&str>,
        header_order: Option<&[String]>,
    ) -> Result<TransportResponse> {
        let Some(host) = url.host_str() else {
            return send_request_auto(
                &self.inner.pool,
                &self.inner.connector,
                &self.inner.h2_config,
                method,
                url,
                headers,
                body,
                proxy,
                false,
                header_order,
            )
            .await;
        };
        let port = url.port_or_known_default().unwrap_or(443);

        enum Winner {
            H3,
            H2,
        }

        let h3_connect = checkout_h3_handle(
            &self.inner.pool,
            h3_config,
            self.inner.profile,
            &self.inner.tls_trust,
            host,
            port,
        );
        let h2_connect = checkout_handle(
            &self.inner.pool,
            &self.inner.connector,
            &self.inner.h2_config,
            host,
            port,
            proxy,
        );
        tokio::pin!(h3_connect, h2_connect);

        let mut h3_done = false;
        let mut h2_done = false;
        let winner = loop {
            tokio::select! {
                r = &mut h3_connect, if !h3_done => match r {
                    Ok(_) => break Some(Winner::H3),
                    Err(_) => {
                        h3_done = true;
                        if h2_done {
                            break None;
                        }
                    }
                },
                r = &mut h2_connect, if !h2_done => match r {
                    Ok(_) => break Some(Winner::H2),
                    Err(_) => {
                        h2_done = true;
                        if h3_done {
                            break None;
                        }
                    }
                },
            }
        };

        match winner {
            Some(Winner::H3) => {
                Box::pin(send_request_h3(
                    &self.inner.pool,
                    h3_config,
                    self.inner.profile,
                    &self.inner.tls_trust,
                    method,
                    url,
                    headers,
                    body,
                    false,
                ))
                .await
            }
            Some(Winner::H2) => {
                send_request_h2(
                    &self.inner.pool,
                    &self.inner.connector,
                    &self.inner.h2_config,
                    method,
                    url,
                    headers,
                    body,
                    proxy,
                    false,
                    header_order,
                )
                .await
            }
            None => {
                send_request_auto(
                    &self.inner.pool,
                    &self.inner.connector,
                    &self.inner.h2_config,
                    method,
                    url,
                    headers,
                    body,
                    proxy,
                    false,
                    header_order,
                )
                .await
            }
        }
    }
}

#[cfg(test)]
mod tests;
