#[cfg(feature = "http3")]
use crate::core::ProtocolPolicy;
use crate::core::ProxyUrl;
use crate::core::error::{Error, Kind, Result};
use crate::core::proxy_pool::ProxyPool;
use crate::profile::{Browser, Platform};

use super::super::proxy::{InvalidEnvProxy, apply_env_proxy};
use super::SessionBuilder;

impl SessionBuilder {
    pub(super) fn validate(&mut self, on_invalid: InvalidEnvProxy) -> Result<()> {
        self.check_config()?;
        self.resolve_platform();
        self.resolve_body_cap();
        #[cfg(feature = "http3")]
        self.resolve_protocol();
        self.resolve_proxies(on_invalid)?;
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

    #[cfg(feature = "http3")]
    fn resolve_protocol(&mut self) {
        if !self.protocol_explicit && self.profile_races_h3() {
            self.protocol_policy = ProtocolPolicy::Race;
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
        } else if let Some(platform) = self.browser.and_then(native_platform) {
            platform
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

    fn resolve_body_cap(&mut self) {
        if let Some(bytes) = self.max_body_size {
            self.compression.max_body_size = bytes;
        }
    }

    fn resolve_proxies(&mut self, on_invalid: InvalidEnvProxy) -> Result<()> {
        for rule in self.proxy_rules() {
            ProxyUrl::parse(rule.url())?;
        }
        #[cfg(not(feature = "socks"))]
        self.check_socks_feature()?;
        self.proxy_config = apply_env_proxy(std::mem::take(&mut self.proxy_config), on_invalid)?;
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
        let socks = self.proxy_rules().find(|rule| {
            url::Url::parse(rule.url())
                .is_ok_and(|url| matches!(url.scheme(), "socks5" | "socks5h"))
        });
        match socks {
            Some(rule) => Err(ProxyUrl::socks_feature_error(rule.url())),
            None => Ok(()),
        }
    }

    pub(super) fn check_profile_id(expected: Option<&str>, actual: Option<&str>) -> Result<()> {
        match expected {
            Some(id) if actual != Some(id) => Err(Error::new(Kind::Config)
                .with_message(format!(
                    "the session profile_id {} differs from the expected {id}",
                    actual.unwrap_or("(none)")
                ))
                .with_source(crate::core::error::ProfileChanged)),
            _ => Ok(()),
        }
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

fn native_platform(browser: Browser) -> Option<Platform> {
    Platform::all().iter().copied().find(|&platform| {
        platform != Platform::Host
            && browser.for_platform(platform) == browser
            && browser.identity(platform, None).is_some()
    })
}
