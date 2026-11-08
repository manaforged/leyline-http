use crate::iana::{ALPN, SERVER_NAME};

pub(crate) const JA3_RECORD_VERSION: u16 = 771;
pub(crate) const JA3_POINT_FORMATS: u16 = 0;
pub(crate) const JA4_TCP: char = 't';
const HASH_LEN: usize = 12;

/// The cipher, extension, and curve ids a raw JA3 string lists, in send order.
#[derive(Debug)]
pub(crate) struct Ja3 {
    pub(crate) ciphers: Vec<u16>,
    pub(crate) extensions: Vec<u16>,
    pub(crate) curves: Vec<u16>,
}

impl Ja3 {
    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
        let fields: Vec<&str> = raw.trim().split(',').collect();
        let [version, ciphers, extensions, curves, point_formats] = fields.as_slice() else {
            return Err(format!(
                "expected 5 comma-separated fields (version,ciphers,extensions,curves,point formats), \
                 got {}",
                fields.len()
            ));
        };
        if *version != JA3_RECORD_VERSION.to_string() {
            return Err(format!(
                "version {version} is not {JA3_RECORD_VERSION}; leyline sends TLS 1.2 as the \
                 ClientHello version"
            ));
        }
        if parse_list(point_formats, 10, "point format")? != [JA3_POINT_FORMATS] {
            return Err(format!(
                "point formats {point_formats:?} are not \"{JA3_POINT_FORMATS}\"; BoringSSL sends \
                 only uncompressed"
            ));
        }
        Ok(Self {
            ciphers: parse_list(ciphers, 10, "cipher")?,
            curves: parse_list(curves, 10, "curve")?,
            extensions: parse_list(extensions, 10, "extension")?,
        })
    }
}

fn is_hash(part: &str) -> bool {
    part.len() == HASH_LEN && part.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The cipher, extension, and signature algorithm ids a raw JA4_r string lists; the extensions start with server_name and ALPN, which JA4_r leaves out.
#[derive(Debug)]
pub(crate) struct Ja4 {
    pub(crate) ciphers: Vec<u16>,
    pub(crate) sigalgs: Vec<u16>,
    pub(crate) extensions: Vec<u16>,
}

impl Ja4 {
    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
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
        if flags.next() != Some(JA4_TCP) {
            return Err(format!(
                "header {header:?} is not a TCP TLS fingerprint (it must start with '{JA4_TCP}')"
            ));
        }
        if flags.nth(2) != Some('d') {
            return Err(format!(
                "header {header:?} has no SNI flag 'd'; leyline always sends server_name"
            ));
        }
        let ciphers = parse_list(ciphers, 16, "cipher")?;
        let sigalgs = parse_list(sigalgs, 16, "signature algorithm")?;
        let mut ids = vec![SERVER_NAME, ALPN];
        ids.extend(parse_list(extensions, 16, "extension")?);
        Ok(Self {
            ciphers,
            sigalgs,
            extensions: ids,
        })
    }
}

fn parse_list(field: &str, radix: u32, what: &str) -> Result<Vec<u16>, String> {
    field
        .split([',', '-'])
        .filter(|item| !item.is_empty())
        .map(|item| {
            u16::from_str_radix(item.trim(), radix)
                .map_err(|e| format!("{what} value {item:?} is not a 16-bit number: {e}"))
        })
        .collect()
}
