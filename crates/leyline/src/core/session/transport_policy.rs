use super::proxy::host_bypasses_proxy;
use super::{ProtocolPolicy, Session};
use crate::core::body::Body;
use crate::core::error::{Error, Result};

impl Session {
    /// Return the session's configured proxy, gated by `NO_PROXY` —
    /// hosts matching a `NO_PROXY` pattern bypass the configured
    /// proxy entirely.
    fn effective_proxy_for(&self, url: &url::Url) -> Option<&str> {
        let proxy = self.proxy.as_deref()?;
        let host = url.host_str().unwrap_or("");
        if host_bypasses_proxy(host) {
            None
        } else {
            Some(proxy)
        }
    }

    pub(super) async fn send_with_policy(
        &self,
        method: &str,
        url: &url::Url,
        headers: Vec<(String, String)>,
        body: Body,
        stream_response: bool,
    ) -> Result<crate::core::transport::TransportResponse> {
        let proxy = self.effective_proxy_for(url);
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
                crate::core::transport::send_request_h1(
                    &self.pool,
                    &self.connector,
                    method,
                    url,
                    headers,
                    body,
                    proxy,
                    stream_response,
                )
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
            ProtocolPolicy::Http3 => {
                if self.proxy.is_some() {
                    return Err(Error::Config(
                        "HTTP/3 over proxies is not implemented; use Auto or Http2".into(),
                    ));
                }
                crate::core::transport::send_request_h3(
                    &self.h3_config,
                    self.profile,
                    method,
                    url,
                    headers,
                    body,
                    stream_response,
                )
                .await
            }
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
                if self.proxy.is_none() && url.scheme() == "https" {
                    // We have a buffered body — clone for the retry.
                    let retained = match &body {
                        Body::Empty => Body::Empty,
                        Body::Bytes(b) => Body::Bytes(b.clone()),
                        Body::Stream { .. } => unreachable!(),
                    };
                    if let Ok(resp) = crate::core::transport::send_request_h3(
                        &self.h3_config,
                        self.profile,
                        method,
                        url,
                        headers.clone(),
                        retained,
                        stream_response,
                    )
                    .await
                    {
                        return Ok(resp);
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
