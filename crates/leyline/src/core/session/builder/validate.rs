#[cfg(feature = "http3")]
use crate::core::Protocol;
use crate::core::proxy_pool::ProxyPool;
use crate::error::{Error, Result};
use crate::profile::Platform;

use super::super::proxy::InvalidEnvProxy;
use super::SessionBuilder;

impl SessionBuilder {
    pub(super) fn validate(&mut self, on_invalid: InvalidEnvProxy) -> Result<()> {
        self.check_config()?;
        if self.refresh_token.is_some() && (self.bearer.is_none() || self.base_url.is_none()) {
            return Err(Error::config(
                "refresh_token requires bearer_auth and base_url",
            ));
        }
        self.check_proxy_pool()?;
        self.resolve_platform();
        #[cfg(feature = "http3")]
        self.resolve_protocol();
        self.resolve_proxies(on_invalid)?;
        #[cfg(feature = "http3")]
        self.check_h3_proxy()?;
        Ok(())
    }

    fn check_config(&mut self) -> Result<()> {
        let Some(error) = self.config_error.take() else {
            return Ok(());
        };
        Err(std::sync::Arc::try_unwrap(error).unwrap_or_else(|shared| {
            Error::config(shared.message().unwrap_or_default().to_owned())
        }))
    }

    fn check_proxy_pool(&self) -> Result<()> {
        if self
            .proxy_pool
            .iter()
            .flat_map(ProxyPool::configs)
            .any(|config| config.pool.is_some())
        {
            return Err(Error::config(
                "a Proxy::list or Proxy::identified entry must be one proxy, not another \
                 Proxy::list or Proxy::identified",
            ));
        }
        if !self.impersonates()
            && self
                .proxy_pool
                .as_ref()
                .is_some_and(ProxyPool::needs_impersonation)
        {
            return Err(Error::config(
                "Proxy::identified pins a browser per proxy and needs a browser \
                 session: add `.browser(..)`, or use Proxy::list",
            ));
        }
        Ok(())
    }

    #[cfg(feature = "http3")]
    fn resolve_protocol(&mut self) {
        if !self.protocol_explicit && self.profile_races_h3() {
            self.protocol_policy = Protocol::Race;
        }
    }

    #[cfg(feature = "http3")]
    fn profile_races_h3(&self) -> bool {
        let h3 = match (&self.profile, self.browser) {
            (Some(profile), _) => profile.h3.as_ref(),
            (None, Some(browser)) => browser.profile().h3.as_ref(),
            (None, None) => None,
        };
        h3.is_some_and(|h3| h3.race)
    }

    fn resolve_platform(&mut self) {
        self.platform = if self.platform_explicit {
            self.platform.resolve()
        } else {
            Platform::detect_host()
        };
    }

    fn resolve_proxies(&mut self, on_invalid: InvalidEnvProxy) -> Result<()> {
        for rule in self.proxy_rules() {
            rule.proxy()?;
        }
        #[cfg(not(feature = "socks"))]
        self.check_socks_feature()?;
        self.proxy_config = std::mem::take(&mut self.proxy_config).apply_env_proxy(on_invalid)?;
        Ok(())
    }

    fn proxy_rules(&self) -> impl Iterator<Item = &crate::core::config::ProxyRule> {
        let pool = self.proxy_pool.iter().flat_map(ProxyPool::configs);
        std::iter::once(&self.proxy_config)
            .chain(pool)
            .flat_map(|config| config.rules().iter())
    }

    #[cfg(not(feature = "socks"))]
    fn check_socks_feature(&self) -> Result<()> {
        let socks = self
            .proxy_rules()
            .find(|rule| rule.proxy().is_ok_and(|proxy| proxy.scheme().is_socks()));
        match socks {
            Some(rule) => Err(Error::config(crate::net::proxy::socks_disabled(rule.url()))),
            None => Ok(()),
        }
    }

    #[cfg(feature = "http3")]
    fn check_h3_proxy(&self) -> Result<()> {
        if self.proxy_config.proxies_every_url()
            && matches!(self.protocol_policy, Protocol::Http3)
            && !crate::quic::proxy_carries_h3(self.proxy_config.primary_url())
        {
            return Err(Error::config(
                "HTTP/3 needs a socks5:// or socks5h:// proxy with UDP ASSOCIATE; http and \
                 https proxies cannot carry QUIC: use another `.protocol(..)` to run HTTP/2 \
                 over the proxy's CONNECT tunnel, or use a SOCKS5 proxy",
            ));
        }
        Ok(())
    }
}
