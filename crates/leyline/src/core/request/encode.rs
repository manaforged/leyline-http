//! URL-encoding and Base64 helpers used by the request builder.

/// URL-encode key-value pairs.
pub(crate) fn url_encode_pairs(params: &[(&str, &str)]) -> String {
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

/// Base64-encode a string for `Authorization: Basic` headers.
///
/// Thin wrapper over `base64::Engine::encode` with the STANDARD alphabet
/// and `=` padding. Exists only so upstream call sites stay string-shaped;
/// prefer the `base64` crate directly when writing new code.
pub(crate) fn base64_encode(input: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(input.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encode_simple() {
        assert_eq!(url_encode("hello world"), "hello+world");
        assert_eq!(url_encode("a=b&c=d"), "a%3Db%26c%3Dd");
        assert_eq!(url_encode("safe-string_v2.0"), "safe-string_v2.0");
    }

    #[test]
    fn url_encode_pairs_works() {
        let pairs = url_encode_pairs(&[("user", "alice"), ("pass", "s3cr3t!")]);
        assert_eq!(pairs, "user=alice&pass=s3cr3t%21");
    }

    #[test]
    fn base64_encode_works() {
        assert_eq!(base64_encode("user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64_encode("a"), "YQ==");
        assert_eq!(base64_encode("ab"), "YWI=");
    }
}
