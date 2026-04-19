//! Proxy-tunnel glue — dispatches to HTTP `CONNECT` or SOCKS5, and
//! owns the helpers shared by both paths (base64, percent-decode).

use crate::tls::connector::FingerprintConnector;
use crate::tls::error::TlsError;
use crate::tls::TlsStream;

pub(crate) mod http;
pub(crate) mod socks5;

/// Dispatch a proxied TLS connection based on the scheme of `proxy_url`.
///
/// `include_alps` matches the direct-path ALPN policy: `true` for h2,
/// `false` for HTTP/1.1-only (WebSocket upgrade). The caller decides
/// which based on the connection it is ultimately establishing.
pub(crate) async fn connect_through_proxy(
    connector: &FingerprintConnector,
    host: &str,
    port: u16,
    proxy_url: &str,
    include_alps: bool,
) -> Result<TlsStream, TlsError> {
    let proxy = url::Url::parse(proxy_url)
        .map_err(|e| TlsError::Profile(format!("invalid proxy URL: {e}")))?;

    if proxy.scheme() == "socks5" || proxy.scheme() == "socks5h" {
        return socks5::connect(connector, host, port, &proxy, include_alps).await;
    }

    http::connect(connector, host, port, &proxy, include_alps).await
}

/// Base64 encode without pulling a dependency — used only for the
/// HTTP `Proxy-Authorization: Basic` header.
pub(super) fn base64_encode(input: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[(n >> 18 & 0x3F) as usize] as char);
        out.push(CHARS[(n >> 12 & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[(n >> 6 & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(n & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Decode percent-encoded URL component (e.g. proxy username/password).
pub(super) fn percent_decode(s: &str) -> String {
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
