use crate::iana::{cipher_name, sigalg_name};
use crate::profile::TlsProfile;
use crate::profile::extension::apply_extensions;

use super::{iana_names, parse_list};

const HASH_LEN: usize = 12;

const SERVER_NAME: u16 = 0x0000;

const ALPN: u16 = 0x0010;

fn is_hash(part: &str) -> bool {
    part.len() == HASH_LEN && part.bytes().all(|b| b.is_ascii_hexdigit())
}

pub(super) fn apply(tls: &mut TlsProfile, raw: &str) -> Result<(), String> {
    let parts: Vec<&str> = raw.trim().split('_').collect();
    let [header, ciphers, extensions, sigalgs] = parts.as_slice() else {
        if parts.len() == 3 && parts[1..].iter().all(|part| is_hash(part)) {
            return Err(
                "this is the hashed JA4 form; its SHA-256 sections cannot be inverted to \
                        cipher and extension lists. Pass the raw JA4_r (or JA4_ro) string"
                    .to_owned(),
            );
        }
        return Err(format!(
            "expected 4 underscore-separated sections (header_ciphers_extensions_sigalgs), got {}",
            parts.len()
        ));
    };
    let mut flags = header.chars();
    if flags.next() != Some('t') {
        return Err(format!(
            "header {header:?} is not a TCP TLS fingerprint (it must start with 't')"
        ));
    }
    if flags.nth(2) != Some('d') {
        return Err(format!(
            "header {header:?} has no SNI flag 'd'; leyline always sends server_name"
        ));
    }
    tls.ciphers = iana_names(
        tls,
        &parse_list(ciphers, 16, "cipher")?,
        cipher_name,
        "cipher",
    )?;
    tls.sigalgs = iana_names(
        tls,
        &parse_list(sigalgs, 16, "signature algorithm")?,
        sigalg_name,
        "signature algorithm",
    )?;
    let mut ids = vec![SERVER_NAME, ALPN];
    ids.extend(parse_list(extensions, 16, "extension")?);
    apply_extensions(tls, &ids)?;
    tls.extension_permutation = None;
    Ok(())
}
