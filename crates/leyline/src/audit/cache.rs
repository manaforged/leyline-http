use crate::audit::{
    ClientHello, Fingerprint, Ja3Input, Ja4Input, Ja4hInput, compute_ja3, compute_ja4,
    compute_ja4h, compute_ja4t,
};
use crate::h2::H2Config;
use crate::profile::TcpProfile;
use crate::profile::{BrowserProfile, JA3_RECORD_VERSION, TlsProfile, akamai_fingerprint};

/// The connection fingerprints of one session, computed once at build and joined with each request's JA4H.
#[derive(Debug, Clone)]
pub(crate) struct AuditTlsCache {
    tcp: Fingerprint,
    quic: Option<Fingerprint>,
}

impl AuditTlsCache {
    /// Computes the TCP fingerprint for `hello` and, with `quic_tls`, the QUIC one.
    pub(crate) fn new(
        profile: &BrowserProfile,
        h2_config: &H2Config,
        tcp: &TcpProfile,
        hello: ClientHello,
        quic_tls: Option<&TlsProfile>,
    ) -> Self {
        Self {
            tcp: Fingerprint {
                h2_fingerprint: akamai_fingerprint(
                    &h2_config.settings,
                    h2_config.initial_connection_window_size,
                    &h2_config.pseudo_order,
                ),
                ja4t: compute_ja4t(tcp),
                ..hello_fingerprint(&profile.tls, hello)
            },
            quic: quic_tls.map(|tls| hello_fingerprint(tls, ClientHello::QuicH3)),
        }
    }

    pub(crate) fn tcp(&self) -> &Fingerprint {
        &self.tcp
    }

    pub(crate) fn for_request(
        &self,
        method: &str,
        version: http::Version,
        headers: &[(String, String)],
    ) -> Fingerprint {
        let hello = match (version, &self.quic) {
            (http::Version::HTTP_3, Some(quic)) => quic,
            _ => &self.tcp,
        };
        Fingerprint {
            ja4h: compute_ja4h(&Ja4hInput {
                method,
                version,
                headers,
            }),
            request_headers: headers.to_vec(),
            ..hello.clone()
        }
    }
}

fn hello_fingerprint(tls: &TlsProfile, hello: ClientHello) -> Fingerprint {
    let extension_ids = tls.extension_ids(hello.carrier());
    Fingerprint {
        ja4: compute_ja4(&Ja4Input {
            ciphers: &tls.ciphers,
            sigalgs: &tls.sigalgs,
            extension_ids: &extension_ids,
            has_sni: true,
            hello,
        }),
        ja3: compute_ja3(&Ja3Input {
            ciphers: &tls.ciphers,
            curves: &tls.curves,
            extension_ids: &extension_ids,
            tls_record_version: JA3_RECORD_VERSION,
        }),
        permutes_extensions: tls.permute_extensions,
        ..Fingerprint::default()
    }
}
