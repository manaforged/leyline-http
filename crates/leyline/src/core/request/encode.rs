//! URL-encoding helpers used by the request builder.

/// URL-encode key-value pairs.
pub(crate) fn url_encode_pairs(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Percent-encode a string for `application/x-www-form-urlencoded`.
///
/// Follows the WHATWG form-urlencoded rules: unreserved set per RFC 3986
/// stays as-is, space becomes `+`, everything else is `%HH`. Matches
/// `percent_encoding::NON_ALPHANUMERIC` with a `' '` → `'+'` pass.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0xF) as usize] as char);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
