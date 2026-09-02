//! HTTP/1.1 `CONNECT` tunnel establishment and response validation.

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::tls::TlsStream;
use crate::tls::error::TlsError;

use crate::util::{base64_encode, percent_decode};

/// Open a TLS-over-HTTP-CONNECT tunnel through a cleartext `http://` proxy and return the wrapped TLS stream.
pub(crate) async fn connect<C: crate::tls::TlsHandshake>(
    connector: &C,
    host: &str,
    port: u16,
    proxy: &url::Url,
    include_alps: bool,
) -> Result<TlsStream, TlsError> {
    let mut tcp_stream = super::connect_to_proxy(connector, proxy, 8080).await?;
    write_connect_and_validate(&mut tcp_stream, host, port, proxy).await?;
    connector
        .do_tls_handshake(tcp_stream, host, include_alps)
        .await
}

/// Open a CONNECT tunnel through an `https://` proxy: the client→proxy leg is itself TLS, so the CONNECT request and any `Proxy-Authorization` credentials travel encrypted (never in cleartext).
pub(crate) async fn connect_via_tls<C: crate::tls::TlsHandshake>(
    connector: &C,
    host: &str,
    port: u16,
    proxy: &url::Url,
    include_alps: bool,
) -> Result<TlsStream, TlsError> {
    if connector.has_origin_tls_identity() {
        return Err(TlsError::Profile(
            "https:// proxy is not supported together with a client certificate or certificate \
             pins: the origin TLS identity must not be presented to the proxy. Use an http:// \
             CONNECT or socks5:// proxy, or drop the client cert / pins."
                .into(),
        ));
    }

    let proxy_host = proxy
        .host_str()
        .ok_or_else(|| TlsError::Profile("https proxy has no host".into()))?;
    let tcp_stream = super::connect_to_proxy(connector, proxy, 443).await?;

    let proxy_tls = connector
        .do_tls_handshake(tcp_stream, proxy_host, false)
        .await?;
    let mut tunnel = proxy_tls.stream;
    write_connect_and_validate(&mut tunnel, host, port, proxy).await?;

    connector
        .do_tls_handshake_nested(tunnel, host, include_alps)
        .await
}

/// Write the `CONNECT host:port` request (with `Proxy-Authorization` when the proxy URL carries credentials) over `stream`, then read and validate the proxy's response.
async fn write_connect_and_validate<S>(
    stream: &mut S,
    host: &str,
    port: u16,
    proxy: &url::Url,
) -> Result<(), TlsError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let connect_req = if let Some(password) = proxy.password() {
        let username = percent_decode(proxy.username());
        let password = percent_decode(password);
        let credentials = base64_encode(&format!("{username}:{password}"));
        format!(
            "CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\nProxy-Authorization: Basic {credentials}\r\n\r\n"
        )
    } else {
        format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n")
    };

    stream
        .write_all(connect_req.as_bytes())
        .await
        .map_err(TlsError::TcpConnect)?;

    let mut response_buf = Vec::with_capacity(1024);
    let mut tmp = [0u8; 256];
    let end_idx = loop {
        let n = stream.read(&mut tmp).await.map_err(TlsError::TcpConnect)?;
        if n == 0 {
            return Err(TlsError::Profile(
                "proxy closed connection before CONNECT response".into(),
            ));
        }
        response_buf.extend_from_slice(&tmp[..n]);
        if response_buf.len() > 8192 {
            return Err(TlsError::Profile("proxy CONNECT response too large".into()));
        }
        if let Some(i) = response_buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };

    validate_connect_response(&response_buf, end_idx)
}

/// Validate a proxy CONNECT response header block.
pub fn validate_connect_response(buf: &[u8], end_idx: usize) -> Result<(), TlsError> {
    if end_idx < 4 || end_idx > buf.len() {
        return Err(TlsError::Profile(format!(
            "proxy CONNECT response validator called with out-of-contract end_idx={end_idx} buf.len={}",
            buf.len()
        )));
    }

    let response = std::str::from_utf8(&buf[..end_idx])
        .map_err(|_| TlsError::Profile("proxy CONNECT response is not UTF-8".into()))?;
    let mut lines = response.split("\r\n");
    let status_line = lines.next().unwrap_or("");

    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    let code = parts.next().unwrap_or("");
    if !matches!(version, "HTTP/1.1" | "HTTP/1.0") || code != "200" {
        return Err(TlsError::Profile(format!(
            "proxy CONNECT failed: {status_line}"
        )));
    }

    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, _)) = line.split_once(':') {
            let name = name.trim();
            if name.eq_ignore_ascii_case("content-length")
                || name.eq_ignore_ascii_case("transfer-encoding")
            {
                return Err(TlsError::Profile(format!(
                    "proxy CONNECT response contains forbidden framing header `{name}` \
                     (RFC 9110 §9.3.6)"
                )));
            }
        }
    }

    if end_idx < buf.len() {
        return Err(TlsError::Profile(
            "proxy CONNECT response carries trailing bytes after headers \
             (possible TLS-stream injection)"
                .into(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests;
