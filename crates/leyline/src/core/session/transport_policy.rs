use super::{ProtocolPolicy, Session};
use crate::core::error::{Error, Kind, Result};
#[cfg(feature = "http3")]
use crate::core::transport::send_request_h3;
use crate::core::transport::{
    Prepared, TransportResponse, send_request_auto, send_request_h1, send_request_h2,
};
use crate::core::{ProxyConfig, ProxyUrl};
use crate::pool::checkout_handle;
#[cfg(feature = "http3")]
use crate::pool::{H3Target, checkout_h3_handle};
#[cfg(feature = "http3")]
use crate::quic::H3Config;

impl Session {
    pub(crate) fn proxy_for<'a>(
        &'a self,
        url: &url::Url,
        request: Option<&'a ProxyConfig>,
    ) -> Result<Option<&'a str>> {
        let proxy = request.unwrap_or(&self.inner.proxy_config).proxy_for(url);
        if let Some(proxy) = proxy {
            ProxyUrl::parse(proxy)?;
        }
        Ok(proxy)
    }

    #[cfg(feature = "http3")]
    fn h3_target<'a>(&'a self, config: &'a H3Config) -> H3Target<'a> {
        H3Target {
            config,
            profile: self.inner.profile,
            trust: &self.inner.tls_trust,
            connector: &self.inner.connector,
        }
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
        let pool = &self.inner.pool;
        let connector = &self.inner.connector;
        let h2_config = &self.inner.h2_config;
        match self.inner.protocol_policy {
            ProtocolPolicy::Auto => send_request_auto(pool, connector, h2_config, req).await,
            ProtocolPolicy::Http1 => Box::pin(send_request_h1(pool, connector, req)).await,
            ProtocolPolicy::Http2 => send_request_h2(pool, connector, h2_config, req).await,
            #[cfg(feature = "http3")]
            ProtocolPolicy::Http3 => {
                if req.proxy.is_some() {
                    return Err(Error::new(Kind::Config).with_message(
                        "HTTP/3 over proxies is not implemented; use Auto or Http2",
                    ));
                }
                let h3_config = self.inner.h3_config.as_ref().ok_or_else(|| {
                    Error::new(Kind::Config)
                        .with_message("this browser profile has no HTTP/3 fingerprint")
                })?;
                Box::pin(send_request_h3(pool, &self.h3_target(h3_config), req)).await
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
                    && req.proxy.is_none()
                    && req.url.scheme() == "https";
                match (raceable, self.inner.h3_config.as_ref()) {
                    (true, Some(h3_config)) => self.send_raced(h3_config, req).await,
                    _ => send_request_auto(pool, connector, h2_config, req).await,
                }
            }
        }
    }

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

        let target = self.h3_target(h3_config);
        let h3_connect = checkout_h3_handle(pool, &target, host, port);
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
                Box::pin(send_request_h3(pool, &target, req)).await
            }
            Some(Winner::H2) => send_request_h2(pool, connector, h2_config, req).await,
            None => send_request_auto(pool, connector, h2_config, req).await,
        }
    }
}

#[cfg(test)]
mod tests;
