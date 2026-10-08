use crate::iana::is_grease;
use crate::profile::TlsProfile;

pub(crate) const PADDING: u16 = 0x0015;

pub(crate) const PRE_SHARED_KEY: u16 = 0x0029;

const ALPS_NEW: u16 = 0x44cd;

const ALPS_OLD: u16 = 0x4469;

#[derive(Clone, Copy)]
enum Switch {
    Always,
    Tls12,
    TcpOnly,
    QuicOnly,
    SessionTickets,
    SupportedGroups,
    SignatureAlgorithms,
    StatusRequest,
    SignedCertTimestamps,
    DelegatedCredentials,
    RecordSizeLimit,
    CompressCertificate,
    ApplicationSettings { new_codepoint: bool },
    TrustAnchors,
    EncryptedClientHello,
}

struct Extension {
    id: u16,
    name: &'static str,
    switch: Switch,
}

const fn ext(id: u16, name: &'static str, switch: Switch) -> Extension {
    Extension { id, name, switch }
}

const EXTENSIONS: &[Extension] = &[
    ext(0x0000, "server_name", Switch::Always),
    ext(0x0017, "extended_master_secret", Switch::Tls12),
    ext(0xff01, "renegotiation_info", Switch::Tls12),
    ext(0x000b, "ec_point_formats", Switch::TcpOnly),
    ext(0x0023, "session_ticket", Switch::SessionTickets),
    ext(0x0010, "alpn", Switch::Always),
    ext(0x0033, "key_share", Switch::Always),
    ext(0x002b, "supported_versions", Switch::Always),
    ext(0x002d, "psk_key_exchange_modes", Switch::Always),
    ext(0x000a, "supported_groups", Switch::SupportedGroups),
    ext(0x000d, "signature_algorithms", Switch::SignatureAlgorithms),
    ext(0x0005, "status_request", Switch::StatusRequest),
    ext(
        0x0012,
        "signed_certificate_timestamp",
        Switch::SignedCertTimestamps,
    ),
    ext(
        0x0022,
        "delegated_credentials",
        Switch::DelegatedCredentials,
    ),
    ext(0x001c, "record_size_limit", Switch::RecordSizeLimit),
    ext(0x001b, "compress_certificate", Switch::CompressCertificate),
    ext(
        ALPS_OLD,
        "application_settings",
        Switch::ApplicationSettings {
            new_codepoint: false,
        },
    ),
    ext(
        ALPS_NEW,
        "application_settings",
        Switch::ApplicationSettings {
            new_codepoint: true,
        },
    ),
    ext(0xca34, "trust_anchors", Switch::TrustAnchors),
    ext(0x0039, "quic_transport_parameters", Switch::QuicOnly),
    ext(
        0xfe0d,
        "encrypted_client_hello",
        Switch::EncryptedClientHello,
    ),
];

const ALWAYS_SENT: &str = "leyline always sends it";

const QUIC_ONLY: &str = "leyline sends it only in the HTTP/3 ClientHello";

const NO_BASE_VALUE: &str =
    "the base profile sets no value for it; pass a base profile whose [tls] table sets one";

impl Switch {
    fn advertised(self, tls: &TlsProfile, quic: bool) -> bool {
        match self {
            Self::Always => true,
            Self::Tls12 => !quic || tls.tls12_extensions,
            Self::TcpOnly => !quic,
            Self::QuicOnly => quic,
            Self::SessionTickets => !quic && tls.session_tickets,
            Self::SupportedGroups => !tls.curves.is_empty(),
            Self::SignatureAlgorithms => !tls.sigalgs.is_empty(),
            Self::StatusRequest => tls.ocsp_stapling,
            Self::SignedCertTimestamps => tls.signed_cert_timestamps,
            Self::DelegatedCredentials => tls.delegated_credentials.is_some(),
            Self::RecordSizeLimit => tls.record_size_limit.is_some(),
            Self::CompressCertificate => !tls.cert_compression.is_empty(),
            Self::ApplicationSettings { new_codepoint } => {
                tls.alps.is_some() && tls.alps_new_codepoint == new_codepoint
            }
            Self::TrustAnchors => tls.request_trust_anchors,
            Self::EncryptedClientHello => tls.ech_grease,
        }
    }

    fn set(self, tls: &mut TlsProfile, base: &TlsProfile, on: bool) -> Result<(), &'static str> {
        match self {
            Self::Always
            | Self::Tls12
            | Self::TcpOnly
            | Self::SupportedGroups
            | Self::SignatureAlgorithms => {
                if !on {
                    return Err(ALWAYS_SENT);
                }
            }
            Self::QuicOnly => {
                if on {
                    return Err(QUIC_ONLY);
                }
            }
            Self::SessionTickets => tls.session_tickets = on,
            Self::StatusRequest => tls.ocsp_stapling = on,
            Self::SignedCertTimestamps => tls.signed_cert_timestamps = on,
            Self::TrustAnchors => tls.request_trust_anchors = on,
            Self::EncryptedClientHello => tls.ech_grease = on,
            Self::DelegatedCredentials => {
                tls.delegated_credentials = carried(on, base.delegated_credentials.clone())?;
            }
            Self::RecordSizeLimit => {
                tls.record_size_limit = carried(on, base.record_size_limit)?;
            }
            Self::CompressCertificate => {
                let list = Some(base.cert_compression.clone()).filter(|list| !list.is_empty());
                tls.cert_compression = carried(on, list)?.unwrap_or_default();
            }
            Self::ApplicationSettings { new_codepoint } => {
                if on {
                    tls.alps = carried(on, base.alps.clone())?;
                    tls.alps_new_codepoint = new_codepoint;
                } else if tls.alps_new_codepoint == new_codepoint {
                    tls.alps = None;
                }
            }
        }
        Ok(())
    }
}

fn carried<T>(on: bool, base: Option<T>) -> Result<Option<T>, &'static str> {
    match (on, base) {
        (false, _) => Ok(None),
        (true, Some(value)) => Ok(Some(value)),
        (true, None) => Err(NO_BASE_VALUE),
    }
}

impl TlsProfile {
    pub(crate) fn advertised_extensions(&self, quic: bool) -> Vec<(u16, &'static str)> {
        EXTENSIONS
            .iter()
            .filter(|ext| ext.switch.advertised(self, quic))
            .map(|ext| (ext.id, ext.name))
            .collect()
    }

    pub(crate) fn apply_extensions(&mut self, ids: &[u16]) -> Result<Vec<u16>, String> {
        let base = self.clone();
        let mut order = Vec::new();
        for (index, &id) in ids.iter().enumerate() {
            if is_grease(id) {
                self.grease = true;
            } else if id == PADDING {
                if index + 1 != ids.len() {
                    return Err(format!(
                        "padding (0x{PADDING:04x}) is at entry {index}; BoringSSL always sends it last"
                    ));
                }
            } else if id == PRE_SHARED_KEY {
                self.pre_shared_key = true;
            } else if EXTENSIONS.iter().any(|ext| ext.id == id) {
                order.push(id);
            } else {
                return Err(format!(
                    "extension 0x{id:04x} ({id}) is not one leyline can send"
                ));
            }
        }
        self.padding = ids.contains(&PADDING);
        for ext in EXTENSIONS {
            ext.switch
                .set(self, &base, order.contains(&ext.id))
                .map_err(|why| format!("{} (0x{:04x}) is missing: {why}", ext.name, ext.id))?;
        }
        Ok(order)
    }
}
