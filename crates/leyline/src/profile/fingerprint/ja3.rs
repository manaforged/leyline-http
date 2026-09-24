use crate::iana::{cipher_name, curve_name};
use crate::profile::TlsProfile;
use crate::profile::extension::apply_extensions;

use super::{iana_names, parse_list};

const RECORD_VERSION: &str = "771";

pub(super) fn apply(tls: &mut TlsProfile, raw: &str) -> Result<(), String> {
    let fields: Vec<&str> = raw.trim().split(',').collect();
    let [version, ciphers, extensions, curves, point_formats] = fields.as_slice() else {
        return Err(format!(
            "expected 5 comma-separated fields (version,ciphers,extensions,curves,point formats), \
             got {}",
            fields.len()
        ));
    };
    if *version != RECORD_VERSION {
        return Err(format!(
            "version {version} is not {RECORD_VERSION}; leyline sends TLS 1.2 as the ClientHello \
             version"
        ));
    }
    if parse_list(point_formats, 10, "point format")? != [0] {
        return Err(format!(
            "point formats {point_formats:?} are not \"0\"; BoringSSL sends only uncompressed"
        ));
    }
    tls.ciphers = iana_names(
        tls,
        &parse_list(ciphers, 10, "cipher")?,
        cipher_name,
        "cipher",
    )?;
    tls.curves = iana_names(tls, &parse_list(curves, 10, "curve")?, curve_name, "curve")?;
    let order = apply_extensions(tls, &parse_list(extensions, 10, "extension")?)?;
    tls.extension_permutation = Some(order);
    tls.permute_extensions = false;
    Ok(())
}
