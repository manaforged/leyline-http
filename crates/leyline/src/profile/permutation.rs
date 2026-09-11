use crate::profile::TlsProfile;

const PRE_SHARED_KEY: u16 = 0x0029;

const ALPS_NEW: u16 = 0x44cd;

const ALPS_OLD: u16 = 0x4469;

fn advertised_extensions(tls: &TlsProfile) -> Vec<(u16, &'static str)> {
    let alps = if tls.alps_new_codepoint {
        ALPS_NEW
    } else {
        ALPS_OLD
    };
    [
        (0x0000, "server_name", true),
        (0x0017, "extended_master_secret", true),
        (0xff01, "renegotiation_info", true),
        (0x000b, "ec_point_formats", true),
        (0x0023, "session_ticket", tls.session_tickets),
        (0x0010, "alpn", true),
        (0x0033, "key_share", true),
        (0x002b, "supported_versions", true),
        (0x002d, "psk_key_exchange_modes", true),
        (0x000a, "supported_groups", !tls.curves.is_empty()),
        (0x000d, "signature_algorithms", !tls.sigalgs.is_empty()),
        (0x0005, "status_request", tls.ocsp_stapling),
        (
            0x0012,
            "signed_certificate_timestamp",
            tls.signed_cert_timestamps,
        ),
        (
            0x0022,
            "delegated_credentials",
            tls.delegated_credentials.is_some(),
        ),
        (0x001c, "record_size_limit", tls.record_size_limit.is_some()),
        (
            0x001b,
            "compress_certificate",
            !tls.cert_compression.is_empty(),
        ),
        (alps, "application_settings", tls.alps.is_some()),
        (0xca34, "trust_anchors", tls.request_trust_anchors),
        (0xfe0d, "encrypted_client_hello", tls.ech_grease),
    ]
    .into_iter()
    .filter_map(|(id, name, advertised)| advertised.then_some((id, name)))
    .collect()
}

pub(crate) fn extension_ids(tls: &TlsProfile) -> Vec<u16> {
    let mut ids = match tls.extension_permutation.as_deref() {
        Some(order) => order.to_vec(),
        None => advertised_extensions(tls)
            .into_iter()
            .map(|(id, _)| id)
            .collect(),
    };
    if tls.padding {
        ids.push(0x0015);
    }
    ids
}

pub(super) fn validate(tls: &TlsProfile) -> Result<(), String> {
    let Some(order) = tls.extension_permutation.as_deref() else {
        return Ok(());
    };
    if order.is_empty() {
        return Err(
            "extension_permutation is empty; omit the key to accept BoringSSL's \
                    default extension order"
                .to_string(),
        );
    }

    let advertised = advertised_extensions(tls);
    for (index, id) in order.iter().enumerate() {
        if *id == PRE_SHARED_KEY {
            return Err(format!(
                "extension_permutation entry {index} is pre_shared_key (0x{PRE_SHARED_KEY:04x}), \
                 which TLS 1.3 fixes last and BoringSSL will not reposition; drop it from the list"
            ));
        }
        if order[..index].contains(id) {
            return Err(format!(
                "extension_permutation repeats 0x{id:04x} at entry {index}; BoringSSL rejects a \
                 duplicated extension ID, leaving the ClientHello in its default order"
            ));
        }
        if !advertised.iter().any(|(known, _)| known == id) {
            return Err(format!(
                "extension_permutation entry {index} is 0x{id:04x}, which this profile never \
                 advertises; order only the extensions the [tls] block turns on"
            ));
        }
    }

    let missing: Vec<String> = advertised
        .iter()
        .filter(|(id, _)| !order.contains(id))
        .map(|(id, name)| format!("{name} (0x{id:04x})"))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "extension_permutation omits {}; BoringSSL appends whatever the list leaves out, in \
             its own order, so the tail of the ClientHello would not match the captured browser. \
             Add each one at its captured position",
            missing.join(", ")
        ));
    }

    Ok(())
}
