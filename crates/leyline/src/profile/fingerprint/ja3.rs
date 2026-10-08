use crate::iana::{cipher_name, curve_name};
use crate::profile::TlsProfile;

use super::parse_list;

const RECORD_VERSION: &str = "771";

impl TlsProfile {
    pub(super) fn apply_ja3(&mut self, raw: &str) -> Result<(), String> {
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
        self.ciphers =
            self.iana_names(&parse_list(ciphers, 10, "cipher")?, cipher_name, "cipher")?;
        self.curves = self.iana_names(&parse_list(curves, 10, "curve")?, curve_name, "curve")?;
        if let Some(shares) = self.key_shares.as_mut() {
            shares.retain(|share| self.curves.contains(share));
        }
        let order = self.apply_extensions(&parse_list(extensions, 10, "extension")?)?;
        self.extension_permutation = Some(order);
        self.permute_extensions = false;
        Ok(())
    }
}
