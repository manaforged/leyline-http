//! Small shared encoding helpers used across the TLS proxy and HTTP layers.
//!
//! These were previously hand-rolled and duplicated in three modules; this is
//! the single source of truth.

/// Standard (padded) base64 encode. Used for HTTP Basic / proxy CONNECT auth.
pub(crate) fn base64_encode(input: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(input.as_bytes())
}

/// Decode a percent-encoded URL component (e.g. proxy username/password).
/// Bytes that are not a valid `%XX` triple are passed through unchanged.
pub(crate) fn percent_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_encode_works() {
        assert_eq!(base64_encode("user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64_encode("a"), "YQ==");
        assert_eq!(base64_encode("ab"), "YWI=");
    }

    #[test]
    fn percent_decode_roundtrips_common_cases() {
        assert_eq!(percent_decode("a%3Db"), "a=b");
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode("s3cr3t%21"), "s3cr3t!");
        // A malformed trailing % is passed through, not dropped.
        assert_eq!(percent_decode("bad%"), "bad%");
    }
}
