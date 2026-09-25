use crate::profile::TlsProfile;
use crate::profile::extension::{PADDING, PRE_SHARED_KEY, advertised_extensions};

pub(crate) fn extension_ids(tls: &TlsProfile) -> Vec<u16> {
    let mut ids = match tls.extension_permutation.as_deref() {
        Some(order) => order.to_vec(),
        None => advertised_extensions(tls, false)
            .into_iter()
            .map(|(id, _)| id)
            .collect(),
    };
    if tls.padding {
        ids.push(PADDING);
    }
    ids
}

pub(super) fn validate(tls: &TlsProfile, quic: bool) -> Result<(), String> {
    validate_tail(tls, quic)?;
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

    let advertised = advertised_extensions(tls, quic);
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

fn validate_tail(tls: &TlsProfile, quic: bool) -> Result<(), String> {
    if tls.extension_tail.is_empty() {
        return Ok(());
    }
    if !quic || tls.grease || !tls.permute_extensions || tls.extension_permutation.is_some() {
        return Err(
            "extension_tail belongs in [h3.tls] with permute_extensions = true, grease = false \
             and no extension_permutation; it pins the last extensions of a shuffled ClientHello"
                .to_string(),
        );
    }
    let advertised = advertised_extensions(tls, quic);
    for (index, id) in tls.extension_tail.iter().enumerate() {
        if *id == PRE_SHARED_KEY || tls.extension_tail[..index].contains(id) {
            return Err(format!(
                "extension_tail entry {index} (0x{id:04x}) is pre_shared_key or a repeat"
            ));
        }
        if !advertised.iter().any(|(known, _)| known == id) {
            return Err(format!(
                "extension_tail entry {index} is 0x{id:04x}, which this profile never advertises"
            ));
        }
    }
    Ok(())
}
