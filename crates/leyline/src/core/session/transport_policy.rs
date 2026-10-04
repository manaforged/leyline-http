use super::{ProtocolPolicy, Session};
use crate::core::error::{Error, Kind, Result};
use crate::core::transport::{
    Prepared, TransportResponse, send_request_auto, send_request_h1, send_request_h2,
};
#[cfg(feature = "http3")]
use crate::core::transport::{ResponseMode, send_request_h3};
use crate::core::{ProxyConfig, ProxyUrl};
#[cfg(feature = "http3")]
use crate::pool::checkout_handle;
use crate::pool::h1::H1Dial;
#[cfg(feature = "http3")]
use crate::pool::{H3Target, checkout_h3_handle};
#[cfg(feature = "http3")]
use crate::quic::{H3Config, h3_proxy_blocker};

#[cfg(feature = "http3")]
static RACE_PROXY_FALLBACK_LOGGED: std::sync::Once = std::sync::Once::new();

#[cfg(feature = "http3")]
fn log_race_proxy_fallback(cause: &'static str) {
    RACE_PROXY_FALLBACK_LOGGED.call_once(|| {
        tracing::debug!(
            target: "leyline::session",
            cause,
            "Race skips HTTP/3 through this proxy and uses Auto"
        );
    });
}

impl Session {
    pub(crate) fn proxy_for<'a>(
        &'a self,
        url: &url::Url,
        request: Option<&'a ProxyConfig>,
    ) -> Result<Option<&'a str>> {
        let proxy = request.unwrap_or(&self.inner.proxy_config).proxy_for(url)?;
        if let Some(proxy) = proxy {
            ProxyUrl::parse(proxy)?;
        }
        Ok(proxy)
    }

    #[cfg(feature = "http3")]
    fn h3_target<'a>(&'a self, config: &'a H3Config) -> H3Target<'a> {
        H3Target {
            config,
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
            ProtocolPolicy::Http1 => {
                Box::pin(send_request_h1(pool, connector, H1Dial::Http1Only, req)).await
            }
            ProtocolPolicy::Http2 => send_request_h2(pool, connector, h2_config, req).await,
            #[cfg(feature = "http3")]
            ProtocolPolicy::Http3 => self.send_h3_only(req).await,
            #[cfg(feature = "http3")]
            ProtocolPolicy::Race => self.send_race_policy(req).await,
        }
    }

    #[cfg(feature = "http3")]
    async fn send_h3_only(&self, req: Prepared<'_>) -> Result<TransportResponse> {
        if let Some(cause) = h3_proxy_blocker(req.proxy) {
            return Err(Error::new(Kind::Config).with_message(cause));
        }
        let h3_config = self.inner.h3_config.as_ref().ok_or_else(|| {
            Error::new(Kind::Config).with_message("this browser profile has no HTTP/3 fingerprint")
        })?;
        Box::pin(send_request_h3(
            &self.inner.pool,
            &self.h3_target(h3_config),
            req,
        ))
        .await
    }

    #[cfg(feature = "http3")]
    async fn send_race_policy(&self, req: Prepared<'_>) -> Result<TransportResponse> {
        match (self.race_wanted(&req), self.inner.h3_config.as_ref()) {
            (true, Some(h3_config)) => self.send_raced(h3_config, req).await,
            _ => {
                send_request_auto(
                    &self.inner.pool,
                    &self.inner.connector,
                    &self.inner.h2_config,
                    req,
                )
                .await
            }
        }
    }

    #[cfg(feature = "http3")]
    fn race_wanted(&self, req: &Prepared<'_>) -> bool {
        if !self.h3_usable(req) {
            return false;
        }
        if let Some(cause) = h3_proxy_blocker(req.proxy) {
            log_race_proxy_fallback(cause);
            return false;
        }
        !req.body.is_stream()
            && req.response != ResponseMode::Streamed
            && req.url.scheme() == "https"
    }

    #[cfg(feature = "http3")]
    fn h3_usable(&self, req: &Prepared<'_>) -> bool {
        let pool = &self.inner.pool;
        req.url
            .host_str()
            .zip(req.url.port_or_known_default())
            .is_some_and(|(host, port)| {
                pool.knows_h3(host, port) && !pool.is_h3_broken(host, port, req.proxy)
            })
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
        let h3_connect = checkout_h3_handle(pool, &target, host, port, req.proxy);
        let h2_connect = checkout_handle(pool, connector, h2_config, host, port, req.proxy);
        tokio::pin!(h3_connect, h2_connect);

        let mut h3_done = false;
        let mut h2_done = false;
        let winner = loop {
            tokio::select! {
                biased;
                r = &mut h3_connect, if !h3_done => match r {
                    Ok(_) => break Some(Winner::H3),
                    Err(error) => {
                        tracing::debug!(
                            target: "leyline::session",
                            host,
                            port,
                            error = %error,
                            "HTTP/3 connection setup failed; Race skips HTTP/3 for this origin and proxy"
                        );
                        pool.note_h3_broken(host, port, req.proxy);
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
            Some(Winner::H3) => Box::pin(send_request_h3(pool, &target, req)).await,
            Some(Winner::H2) => send_request_h2(pool, connector, h2_config, req).await,
            None => send_request_auto(pool, connector, h2_config, req).await,
        }
    }
}

#[cfg(test)]
mod tests;
