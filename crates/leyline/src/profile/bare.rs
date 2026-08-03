//! The synthetic **bare** profile — a hand-built, non-impersonating default.
//!
//! Split out of [`super::types`] because it is the one profile constructed in
//! code rather than deserialized from TOML: keeping it here holds `types.rs`
//! to the deserialized schema and its accessors.

use std::collections::HashMap;

use super::types::{BrowserProfile, H2Profile, PlatformIdentity, ProfileMeta, TlsProfile};

impl BrowserProfile {
    /// The synthetic **bare** profile: a plain, non-impersonating HTTP
    /// client. This is what a [`Session`](crate::Session) is by default when
    /// no `.browser(...)` is chosen — a generic `leyline/<version>`
    /// User-Agent, no `sec-ch-ua`/brand headers, a minimal modern TLS
    /// cipher/curve/sigalg set with none of the browser-specific quirks
    /// (no ALPS, no cert-compression advert, no extension permutation,
    /// no delegated credentials, no record-size-limit), and stock HTTP/2
    /// SETTINGS. It carries no expected fingerprint — there is nothing to
    /// verify against an external browser.
    ///
    /// Built in code rather than from TOML so the default path needs no
    /// profile file and no parse step.
    pub fn bare() -> Self {
        // Generic, OS-agnostic identity. The UA is the same on every
        // platform; only the TCP fingerprint follows the resolved host.
        let identity = PlatformIdentity {
            user_agent: concat!("leyline/", env!("CARGO_PKG_VERSION")).to_string(),
            sec_ch_ua: String::new(),
            accept_language: Some("en-US,en;q=0.9".to_string()),
            request_header_order: None,
            extra_headers: Vec::new(),
            navigate_accept_override: None,
        };
        let mut identity_map = HashMap::new();
        for key in ["windows", "macos", "linux", "android", "ios"] {
            identity_map.insert(key.to_string(), identity.clone());
        }

        BrowserProfile {
            meta: ProfileMeta {
                name: "Bare (no impersonation)".to_string(),
                browser: "bare".to_string(),
                version: 0,
                family: "bare".to_string(),
                // Not anchored to any browser — exempt from verification.
                verified_against: "n/a (synthetic non-impersonating profile)".to_string(),
                // Nothing was captured; the bare client impersonates no build.
                captured_against: None,
            },
            tls: TlsProfile {
                // Minimal modern set; names match the IANA-style spelling the
                // other profiles use (accepted by BoringSSL's cipher parser).
                ciphers: vec![
                    "TLS_AES_128_GCM_SHA256".into(),
                    "TLS_AES_256_GCM_SHA384".into(),
                    "TLS_CHACHA20_POLY1305_SHA256".into(),
                    "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256".into(),
                    "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256".into(),
                    "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384".into(),
                    "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384".into(),
                    "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256".into(),
                    "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256".into(),
                ],
                curves: vec!["X25519".into(), "SECP256R1".into(), "SECP384R1".into()],
                sigalgs: vec![
                    "ecdsa_secp256r1_sha256".into(),
                    "rsa_pss_rsae_sha256".into(),
                    "rsa_pkcs1_sha256".into(),
                    "ecdsa_secp384r1_sha384".into(),
                    "rsa_pss_rsae_sha384".into(),
                    "rsa_pkcs1_sha384".into(),
                ],
                delegated_credentials: None,
                record_size_limit: None,
                alps: None,
                alps_new_codepoint: false,
                ocsp_stapling: false,
                signed_cert_timestamps: false,
                cert_compression: Vec::new(),
                permute_extensions: false,
                extension_permutation: None,
                grease: false,
                ech_grease: false,
                pre_shared_key: false,
                request_trust_anchors: false,
                fingerprint: None,
            },
            h2: H2Profile {
                header_table_size: Some(4096),
                enable_push: Some(false),
                max_concurrent_streams: Some(100),
                initial_stream_window_size: Some(65535),
                initial_connection_window_size: Some(65535),
                max_frame_size: Some(16384),
                max_header_list_size: None,
                unknown_setting8: None,
                unknown_setting9: None,
                pseudo_order: vec![
                    "method".into(),
                    "scheme".into(),
                    "authority".into(),
                    "path".into(),
                ],
                settings_order: vec![
                    "header_table_size".into(),
                    "enable_push".into(),
                    "max_concurrent_streams".into(),
                    "initial_window_size".into(),
                    "max_frame_size".into(),
                ],
                fingerprint: None,
                platforms: HashMap::new(),
            },
            identity: identity_map,
        }
    }
}
