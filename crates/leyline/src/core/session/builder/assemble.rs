use std::sync::Arc;

use crate::profile::preset::{HeaderContext, HeaderPlan};
use crate::tls::FingerprintConnector;

use super::super::{Session, SessionInner};
use super::SessionBuilder;
use super::derive::DerivedIdentity;

impl SessionBuilder {
    pub(super) fn into_session(
        self,
        derived: DerivedIdentity,
        connector: FingerprintConnector,
    ) -> Session {
        let inputs = Arc::new(self.clone());
        let impersonates = self.impersonates();
        let pool = Arc::new(self.build_pool());
        let cookie_jar = self.cookie_jar.unwrap_or_default();
        let user_agent = derived.user_agent.into();
        let sec_ch_ua = derived.sec_ch_ua.into();
        let accept_language = derived.accept_language.into();
        let header_plan = HeaderPlan::shared(
            derived.header_style,
            &HeaderContext::session(&user_agent, &sec_ch_ua, &accept_language, derived.platform),
            self.compression.accept_encoding(),
        );

        Session {
            inner: Arc::new(SessionInner {
                browser: derived.browser,
                impersonates,
                header_plan,
                identity: derived.identity,
                platform: derived.platform,
                brand: self.brand,
                user_agent,
                sec_ch_ua,
                accept_language,
                header_order: derived.header_order,
                default_headers: self
                    .default_headers
                    .into_iter()
                    .map(|(name, value)| (name, crate::header_str::HeaderStr::from(value)))
                    .collect(),
                bearer: self.bearer,
                proxy_shown: self.proxy_config.primary().map(crate::util::redact),
                proxy_config: self.proxy_config,
                timeouts: self.timeouts,
                redirect_policy: self.redirect_policy,
                compression: self.compression,
                #[cfg(feature = "websocket")]
                websocket_config: self.websocket_config,
                https_only: self.https_only,
                cookie_jar,
                connector,
                h2_config: derived.h2_config,
                pool,
                shutdown: Arc::default(),
                writers: Arc::default(),
                audit_tls: derived.audit_tls,
                protocol_policy: self.protocol_policy,
                default_retry: self.default_retry,
                trace: self.trace,
                tls_trust: self.tls_trust.clone(),
                #[cfg(feature = "http3")]
                h3_config: derived.h3_config,
                profile: derived.profile,
                base_url: self.base_url,
                languages: self.languages,
                host_limits: self.host_limits.armed(),
                #[cfg(feature = "html")]
                crawl_limits: Arc::default(),
                #[cfg(feature = "html")]
                crawl_pace: None,
                proxy_pool: self.proxy_pool,
                block_rules: self.block_rules,
                inputs,
            }),
        }
    }
}
