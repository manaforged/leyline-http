use super::{ProtocolPolicy, Session};
use crate::core::error::{Error, Kind, Result};
#[cfg(feature = "tower")]
use crate::core::layer::Call;
#[cfg(feature = "http3")]
use crate::core::transport::send_request_h3;
use crate::core::transport::{
    Prepared, TransportResponse, send_request_auto, send_request_h1, send_request_h2,
};
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
        self.inner.proxy_config.proxy_for(url, request_proxy)
    }

    /// `true` iff *any* proxy was requested for this call (session default OR per-request override) — regardless of `NO_PROXY` filtering.
    #[cfg(feature = "http3")]
    fn proxy_requested(&self, request_proxy: Option<&str>) -> bool {
        request_proxy.is_some() || self.inner.proxy_config.primary().is_some()
    }

    /// Run one prepared attempt: through the middleware stack when the session has one, straight to the transport otherwise.
    pub(crate) async fn dispatch<'a>(&'a self, req: Prepared<'a>) -> Result<TransportResponse> {
        #[cfg(feature = "tower")]
        if let Some(stack) = self.inner.layer.clone() {
            if self.inner.https_only && req.url.scheme() != "https" {
                return Err(Error::new(Kind::Config)
                    .with_message("https_only session rejected non-HTTPS URL"));
            }
            let url = req.url.clone();
            let req = Prepared {
                proxy: self.effective_proxy_for(req.url, req.proxy),
                ..req
            };
            let call = Call::new(self.clone(), req)?;
            return Ok(stack.call(call).await?.seal(&url));
        }
        self.send_with_policy(req).await
    }

    pub(crate) async fn send_with_policy<'a>(
        &'a self,
        req: Prepared<'a>,
    ) -> Result<TransportResponse> {
        if self.inner.https_only && req.url.scheme() != "https" {
            return Err(
                Error::new(Kind::Config).with_message("https_only session rejected non-HTTPS URL")
            );
        }
        let req = Prepared {
            proxy: self.effective_proxy_for(req.url, req.proxy),
            ..req
        };
        let pool = &self.inner.pool;
        let connector = &self.inner.connector;
        let h2_config = &self.inner.h2_config;
        match self.inner.protocol_policy {
            ProtocolPolicy::Auto => send_request_auto(pool, connector, h2_config, req).await,
            ProtocolPolicy::Http1 => Box::pin(send_request_h1(pool, connector, req)).await,
            ProtocolPolicy::Http2 => send_request_h2(pool, connector, h2_config, req).await,
            #[cfg(feature = "http3")]
            ProtocolPolicy::Http3 => {
                if self.proxy_requested(req.proxy) {
                    return Err(Error::new(Kind::Config).with_message(
                        "HTTP/3 over proxies is not implemented; use Auto or Http2",
                    ));
                }
                let h3_config = self.inner.h3_config.as_ref().ok_or_else(|| {
                    Error::new(Kind::Config)
                        .with_message("this browser profile has no HTTP/3 fingerprint")
                })?;
                Box::pin(send_request_h3(
                    pool,
                    h3_config,
                    self.inner.profile,
                    &self.inner.tls_trust,
                    req,
                ))
                .await
            }
            #[cfg(feature = "http3")]
            ProtocolPolicy::Race => {
                let known_h3 = req
                    .url
                    .host_str()
                    .zip(req.url.port_or_known_default())
                    .is_some_and(|(host, port)| pool.knows_h3(host, port));
                let raceable = known_h3
                    && !req.body.is_stream()
                    && !req.stream_response
                    && !self.proxy_requested(req.proxy)
                    && req.url.scheme() == "https";
                match (raceable, self.inner.h3_config.as_ref()) {
                    (true, Some(h3_config)) => self.send_raced(h3_config, req).await,
                    _ => send_request_auto(pool, connector, h2_config, req).await,
                }
            }
        }
    }

    /// Race QUIC against TCP+TLS; the first transport to hand back a connection carries the request.
    #[cfg(feature = "http3")]
    async fn send_raced(
        &self,
        h3_config: &H3Config,
        req: Prepared<'_>,
    ) -> Result<TransportResponse> {
        let pool = &self.inner.pool;
        let connector = &self.inner.connector;
        let h2_config = &self.inner.h2_config;
        let Some(host) = req.url.host_str() else {
            return send_request_auto(pool, connector, h2_config, req).await;
        };
        let port = req.url.port_or_known_default().unwrap_or(443);

        enum Winner {
            H3,
            H2,
        }

        let h3_connect = checkout_h3_handle(
            pool,
            h3_config,
            self.inner.profile,
            &self.inner.tls_trust,
            host,
            port,
        );
        let h2_connect = checkout_handle(pool, connector, h2_config, host, port, req.proxy);
        tokio::pin!(h3_connect, h2_connect);

        let mut h3_done = false;
        let mut h2_done = false;
        let winner = loop {
            tokio::select! {
                biased;
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
                pool.note_h3(host, port);
                Box::pin(send_request_h3(
                    pool,
                    h3_config,
                    self.inner.profile,
                    &self.inner.tls_trust,
                    req,
                ))
                .await
            }
            Some(Winner::H2) => send_request_h2(pool, connector, h2_config, req).await,
            None => send_request_auto(pool, connector, h2_config, req).await,
        }
    }
}

#[cfg(test)]
mod tests;
