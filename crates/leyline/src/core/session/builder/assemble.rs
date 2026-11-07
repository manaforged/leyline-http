use std::sync::Arc;

use crate::profile::preset::{HeaderContext, HeaderPlan};
use crate::tls::FingerprintConnector;

use super::super::cache::Cache;
use super::super::{Session, SessionInner};
use super::SessionBuilder;
use super::derive::DerivedIdentity;

impl SessionBuilder {
    pub(super) fn into_session(
        mut self,
        derived: DerivedIdentity,
        connector: FingerprintConnector,
    ) -> Session {
        let impersonates = self.impersonates();
        let pool = Arc::new(self.pool_inputs().pool(impersonates));
        self.cookie_jar.get_or_insert_default();
        self.bearer = self
            .bearer
            .take()
            .map(|token| token.refreshing(self.refresh_token.clone()));
        self.layered = false;
        let user_agent = derived.user_agent.into();
        let sec_ch_ua = derived.sec_ch_ua.into();
        let accept_language = derived.accept_language.into();
        let header_plan = HeaderPlan::shared(
            derived.header_style,
            &HeaderContext::session(&user_agent, &sec_ch_ua, &accept_language, derived.platform),
            self.compression.accept_encoding(),
        );

        let mut inner = SessionInner {
            browser: derived.browser,
            impersonates,
            header_plan,
            identity: derived.identity,
            platform: derived.platform,
            user_agent,
            sec_ch_ua,
            accept_language,
            header_order: derived.header_order,
            default_headers: self
                .default_headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        crate::header_str::HeaderStr::from(value.as_str()),
                    )
                })
                .collect(),
            cache: (self.cache > 0).then(|| Arc::new(Cache::new(self.cache))),
            proxy_shown: self.proxy_config.primary().map(crate::util::redact),
            connector,
            h2_config: derived.h2_config,
            pool,
            hsts: Arc::default(),
            shutdown: Arc::default(),
            writers: Arc::default(),
            audit_tls: derived.audit_tls,
            #[cfg(feature = "http3")]
            h3_config: derived.h3_config,
            profile: derived.profile,
            host_limits: crate::core::config::HostGates::new(self.host_limits.clone()),
            extra_gates: None,
            header_id: crate::core::request_headers::HeaderId::default(),
            inputs: self,
        };
        inner.pool = Arc::new(inner.pool.shared_view(inner.profile_hash()));
        Session {
            inner: Arc::new(inner),
        }
    }

    pub(in crate::core::session) fn jar(&self) -> &crate::cookie::Jar {
        self.cookie_jar
            .as_ref()
            .expect("invariant: a built session stores its cookie jar")
    }
}
