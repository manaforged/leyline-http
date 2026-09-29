use std::sync::{Arc, Mutex};

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
        let impersonates = self.impersonates();
        let pool = Arc::new(self.build_pool());
        let cookie_jar = self.cookie_jar.unwrap_or_default();

        Session {
            inner: Arc::new(SessionInner {
                browser: derived.browser,
                impersonates,
                header_style: derived.header_style,
                identity: derived.identity,
                platform: derived.platform,
                brand: self.brand,
                user_agent: derived.user_agent,
                sec_ch_ua: derived.sec_ch_ua,
                accept_language: derived.accept_language,
                header_order: derived.header_order,
                default_headers: self.default_headers,
                proxy_config: self.proxy_config,
                timeouts: self.timeouts,
                redirect_policy: self.redirect_policy,
                compression: self.compression,
                #[cfg(feature = "websocket")]
                websocket_config: self.websocket_config,
                https_only: self.https_only,
                cookie_jar,
                url_cache: Arc::new(Mutex::new(None)),
                connector,
                h2_config: derived.h2_config,
                pool,
                audit_tls: derived.audit_tls,
                protocol_policy: self.protocol_policy,
                default_retry: self.default_retry,
                trace: self.trace,
                #[cfg(feature = "http3")]
                tls_trust: self.tls_trust.clone(),
                #[cfg(feature = "http3")]
                h3_config: derived.h3_config,
                profile: derived.profile,
            }),
        }
    }
}
