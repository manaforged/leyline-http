use crate::audit::AuditTlsCache;
#[cfg(feature = "http3")]
use crate::core::ProtocolPolicy;
use crate::core::error::{Error, Result};
use crate::h2::H2Config;
use crate::pool::Pool;
use crate::profile::BrowserProfile;
use crate::tcp::TcpProfile;
use crate::tls::FingerprintConnector;

use super::SessionBuilder;

impl SessionBuilder {
    pub(super) fn compute_audit_cache(
        &self,
        profile: &'static BrowserProfile,
        h2_config: &H2Config,
        tcp_profile: &TcpProfile,
    ) -> AuditTlsCache {
        let extension_ids = crate::audit::extension_ids(&profile.tls);
        let ja4 = {
            let input = crate::audit::Ja4Input {
                ciphers: &profile.tls.ciphers,
                sigalgs: &profile.tls.sigalgs,
                curves: &profile.tls.curves,
                extension_ids: &extension_ids,
                tls_version: "1.3",
                has_sni: true,
                alpn: "h2",
            };
            crate::audit::compute_ja4(&input)
        };
        let ja3 = {
            let input = crate::audit::Ja3Input {
                ciphers: &profile.tls.ciphers,
                curves: &profile.tls.curves,
                extension_ids: &extension_ids,
                tls_record_version: 771,
            };
            crate::audit::compute_ja3(&input)
        };
        let h2_fp = h2_config.akamai_fingerprint();
        let ja4t = crate::audit::compute_ja4t(tcp_profile);
        AuditTlsCache {
            ja4,
            ja3,
            h2_fingerprint: h2_fp,
            ja4t,
        }
    }

    pub(super) fn build_connector(
        &self,
        profile: &'static BrowserProfile,
        tcp_profile: &TcpProfile,
    ) -> Result<FingerprintConnector> {
        let accept_invalid_certs = self.tls_trust.accepts_invalid_certs();
        let tls_trust = if accept_invalid_certs {
            self.tls_trust.clone().system_roots(false)
        } else {
            self.tls_trust.clone()
        };
        let mut fp = FingerprintConnector::new_with_trust(profile, tcp_profile.clone(), &tls_trust)
            .map_err(Error::from)?;
        if accept_invalid_certs {
            fp.set_accept_invalid_certs(true);
        }
        fp = fp.with_resolver(self.dns_config.clone().into_resolver());
        fp = fp.with_socket_config(self.socket_config.clone());
        if let Some(connect_timeout) = self.timeouts.connect_limit() {
            fp = fp.with_connect_timeout(connect_timeout);
        }
        if let Some(config) = self.socket_config.happy_eyeballs {
            fp = fp.with_happy_eyeballs_config(config);
        }
        Ok(fp)
    }

    #[cfg(feature = "http3")]
    pub(super) fn h3_config(
        &self,
        profile: &BrowserProfile,
    ) -> Result<Option<crate::quic::H3Config>> {
        match crate::quic::H3Config::from_profile(profile) {
            Ok(mut cfg) => {
                cfg.max_response_body_bytes = self.compression.max_body_size as u64;
                Ok(Some(cfg))
            }
            Err(e)
                if matches!(
                    self.protocol_policy,
                    ProtocolPolicy::Http3 | ProtocolPolicy::Race
                ) =>
            {
                Err(e)
            }
            Err(_) => Ok(None),
        }
    }

    pub(super) fn build_pool(&self) -> Pool {
        let config = &self.pool_config;
        let (idle_timeout, max_connections) = if config.keepalive {
            (config.idle_timeout, config.max_connections.max(1))
        } else {
            (std::time::Duration::ZERO, 1)
        };
        Pool::with_limits(
            idle_timeout,
            max_connections,
            config.max_h1_conns_per_host.max(1),
        )
        .with_h2_ping(config.h2_ping_after_idle, config.h2_ping_timeout)
        .with_max_body_size(self.compression.max_body_size)
    }
}
