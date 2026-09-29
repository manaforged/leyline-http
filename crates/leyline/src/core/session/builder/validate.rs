#[cfg(feature = "http3")]
use crate::core::ProtocolPolicy;
use crate::core::ProxyUrl;
use crate::core::error::{Error, Kind, Result};
use crate::profile::Platform;

use super::super::proxy::env_proxy;
use super::SessionBuilder;

impl SessionBuilder {
    pub(super) fn validate(&mut self) -> Result<()> {
        self.check_config()?;
        self.resolve_platform();
        self.resolve_proxies()?;
        #[cfg(feature = "http3")]
        self.check_h3_proxy()?;
        Ok(())
    }

    fn check_config(&mut self) -> Result<()> {
        match self.config_error.take() {
            Some(error) => Err(Error::new(Kind::Config).with_message(error)),
            None => Ok(()),
        }
    }

    fn resolve_platform(&mut self) {
        self.platform = if self.platform_explicit {
            self.platform.resolve()
        } else if self.impersonates() {
            static NOTICE: std::sync::Once = std::sync::Once::new();
            NOTICE.call_once(|| {
                tracing::info!(
                    target: "leyline::session",
                    "no .platform() set on an impersonation profile: defaulting to \
                     Windows; call .platform(...) to pin the OS identity"
                );
            });
            Platform::Windows
        } else {
            Platform::detect_host()
        };
    }

    fn resolve_proxies(&mut self) -> Result<()> {
        for rule in self.proxy_config.rules() {
            ProxyUrl::parse(rule.url())?;
        }
        if self.proxy_config.rules().is_empty()
            && self.proxy_config.uses_env()
            && let Some(p) = env_proxy()
        {
            self.proxy_config = self
                .proxy_config
                .clone()
                .set_default_proxy(p)
                .set_from_env();
        }
        Ok(())
    }

    #[cfg(feature = "http3")]
    fn check_h3_proxy(&self) -> Result<()> {
        if self.proxy_config.proxies_every_url()
            && matches!(self.protocol_policy, ProtocolPolicy::Http3)
            && !crate::quic::proxy_carries_h3(self.proxy_config.primary())
        {
            return Err(Error::new(Kind::Config).with_message(
                "HTTP/3 needs a socks5:// or socks5h:// proxy with UDP ASSOCIATE; http and \
                 https proxies cannot carry QUIC: use another `.protocol(..)` to run HTTP/2 \
                 over the proxy's CONNECT tunnel, or use a SOCKS5 proxy",
            ));
        }
        Ok(())
    }
}
