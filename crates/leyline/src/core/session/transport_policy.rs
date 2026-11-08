use super::{Plan, Session};
use crate::core::config::Proxy;
use crate::error::{Error, Result};
use crate::net::ProxyUrl;
use crate::pool::Prepared;
#[cfg(feature = "http3")]
use crate::pool::Route;
#[cfg(feature = "http3")]
use crate::pool::send_h3;
#[cfg(feature = "http3")]
use crate::pool::{H3Target, checkout_h3_handle};
#[cfg(feature = "http3")]
use crate::pool::{OriginFlag, checkout_handle};
#[cfg(feature = "http3")]
use crate::quic::{H3Config, h3_proxy_blocker};
#[cfg(feature = "http3")]
use crate::wire::ResponseMode;
use crate::wire::TransportResponse;
use futures_util::future::Either;
use std::future::Future;
#[cfg(feature = "http3")]
use std::ops::ControlFlow;

#[cfg(feature = "http3")]
type Raced<'r> = ControlFlow<Result<Box<TransportResponse>>, (Route, Prepared<'r>)>;

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
        request: Option<&'a Proxy>,
    ) -> Result<Option<&'a ProxyUrl>> {
        request
            .unwrap_or(&self.inner.inputs.proxy_config)
            .proxy_for(url)
    }

    #[cfg(feature = "http3")]
    fn h3_target<'a>(&'a self, config: &'a H3Config) -> H3Target<'a> {
        H3Target {
            config,
            trust: &self.inner.inputs.tls_trust,
            connector: &self.inner.connector,
        }
    }

    pub(crate) fn send_with_policy<'a>(
        &'a self,
        req: Prepared<'a>,
    ) -> impl Future<Output = Result<Box<TransportResponse>>> + 'a {
        if self.inner.inputs.https_only
            && crate::wire::Scheme::of(req.url) != Some(crate::wire::Scheme::Https)
        {
            return Either::Left(std::future::ready(Err(Error::config(
                "https_only session rejected non-HTTPS URL",
            ))));
        }
        Either::Right(match &self.inner.cache {
            Some(cache) => Either::Left(Box::pin(self.send_cached(req, cache))),
            None => Either::Right(self.send_transport(req)),
        })
    }

    pub(super) async fn send_transport(&self, req: Prepared<'_>) -> Result<Box<TransportResponse>> {
        let (route, req) = match self.inner.inputs.protocol_policy.plan() {
            Plan::Tcp(route) => (route, req),
            #[cfg(feature = "http3")]
            Plan::H3Only => return self.send_h3_only(req).await,
            #[cfg(feature = "http3")]
            Plan::H3First => match self.send_race_policy(req).await {
                ControlFlow::Break(response) => return response,
                ControlFlow::Continue(fallback) => fallback,
            },
        };
        crate::pool::send(
            &self.inner.pool,
            &self.inner.connector,
            &self.inner.h2_config,
            route,
            req,
        )
        .await
    }

    #[cfg(feature = "http3")]
    async fn send_h3_only(&self, req: Prepared<'_>) -> Result<Box<TransportResponse>> {
        if let Some(cause) = h3_proxy_blocker(req.proxy) {
            return Err(Error::config(cause));
        }
        let h3_config = self
            .inner
            .h3_config
            .as_ref()
            .ok_or_else(|| Error::config("this browser profile has no HTTP/3 fingerprint"))?;
        Box::pin(send_h3(&self.inner.pool, &self.h3_target(h3_config), req)).await
    }

    #[cfg(feature = "http3")]
    async fn send_race_policy<'r>(&self, req: Prepared<'r>) -> Raced<'r> {
        match (self.race_wanted(&req), self.inner.h3_config.as_ref()) {
            (true, Some(h3_config)) => Box::pin(self.send_raced(h3_config, req)).await,
            _ => ControlFlow::Continue((Route::Auto, req)),
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
            && crate::wire::Scheme::of(req.url) == Some(crate::wire::Scheme::Https)
    }

    #[cfg(feature = "http3")]
    fn h3_usable(&self, req: &Prepared<'_>) -> bool {
        let pool = &self.inner.pool;
        crate::wire::host_port(req.url)
            .ok()
            .is_some_and(|(host, port)| {
                pool.knows_h3(host, port)
                    && !pool.recalls(OriginFlag::H3Broken, host, port, req.proxy)
            })
    }

    #[cfg(feature = "http3")]
    async fn send_raced<'r>(&self, h3_config: &H3Config, req: Prepared<'r>) -> Raced<'r> {
        let pool = &self.inner.pool;
        let connector = &self.inner.connector;
        let h2_config = &self.inner.h2_config;
        let Ok((host, port)) = crate::wire::host_port(req.url) else {
            return ControlFlow::Continue((Route::Auto, req));
        };

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
                        pool.remember(OriginFlag::H3Broken, host, port, req.proxy);
                        h3_done = true;
                        if h2_done {
                            break None;
                        }
                    }
                },
                r = &mut h2_connect, if !h2_done => match r {
                    Ok(Some(_)) => break Some(Winner::H2),
                    Ok(None) | Err(_) => {
                        h2_done = true;
                        if h3_done {
                            break None;
                        }
                    }
                },
            }
        };

        match winner {
            Some(Winner::H3) => ControlFlow::Break(Box::pin(send_h3(pool, &target, req)).await),
            Some(Winner::H2) => ControlFlow::Continue((Route::Http2, req)),
            None => ControlFlow::Continue((Route::Auto, req)),
        }
    }
}

#[cfg(test)]
mod tests;
