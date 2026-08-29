//! Small shared encoding helpers used across the TLS proxy and HTTP layers.
//!
//! Single source of truth for these encodings, shared by every module that
//! needs them.

#![forbid(unsafe_code)]
// This module must stay free of `unsafe`; memory-unsafe code is confined to
// leyline-bssl* (FFI) and leyline's tcp/tls platform bridges.
/// Redact a URL for logs and error values: the password, if any, becomes
/// `REDACTED`. The username is kept (operators use it to tell accounts
/// apart); the password never belongs in a trace or an error. Unparseable
/// input is returned as-is.
pub(crate) fn redacted_url(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(mut url) if url.password().is_some() => {
            // Plain token: the url crate percent-encodes brackets, which
            // would make the marker noisy in logs.
            let _ = url.set_password(Some("REDACTED"));
            url.to_string()
        }
        _ => raw.to_string(),
    }
}

/// Standard (padded) base64 encode. Used for HTTP Basic / proxy CONNECT auth.
pub(crate) fn base64_encode(input: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(input.as_bytes())
}

/// Generate a random lowercase-hex token from `bytes` cryptographically random
/// bytes (so the output is `2 * bytes` hex chars).
///
/// Shared by the digest `cnonce` and the multipart boundary, which both want a
/// fresh, unpredictable hex string from the process RNG via `rand::thread_rng`.
pub(crate) fn random_hex_token(bytes: usize) -> String {
    use rand::RngCore;
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
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
mod tests;
