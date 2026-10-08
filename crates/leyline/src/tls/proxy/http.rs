use tokio::io::{AsyncReadExt, AsyncWriteExt};

use tokio::net::TcpStream;

use crate::tls::error::{ProxyReply, TlsError};
use crate::tls::{SessionCache, TlsIo, TlsStream};

use crate::util::proxy_basic_auth;

const TARGET_UNREACHABLE_STATUSES: [u16; 2] = [502, 504];

pub(crate) async fn tunnel<C: crate::tls::TlsHandshake>(
    connector: &C,
    host: &str,
    port: u16,
    proxy: &url::Url,
) -> Result<TcpStream, TlsError> {
    let mut tcp_stream = super::connect_to_proxy(connector, proxy).await?;
    write_connect_and_validate(&mut tcp_stream, host, port, proxy).await?;
    Ok(tcp_stream)
}

pub(crate) async fn tunnel_via_tls<C: crate::tls::TlsHandshake>(
    connector: &C,
    host: &str,
    port: u16,
    proxy: &url::Url,
) -> Result<TlsIo, TlsError> {
    let mut tunnel = open_tls_to_proxy(connector, proxy).await?.stream;
    write_connect_and_validate(&mut tunnel, host, port, proxy).await?;
    Ok(tunnel)
}

pub(crate) async fn open_tls_to_proxy<C: crate::tls::TlsHandshake>(
    connector: &C,
    proxy: &url::Url,
) -> Result<TlsStream, TlsError> {
    if connector.has_origin_tls_identity() {
        return Err(TlsError::proxy(
            "https:// proxy is not supported together with a client certificate or certificate \
             pins: the origin TLS identity must not be presented to the proxy. Use an http:// \
             CONNECT or socks5:// proxy, or drop the client cert / pins.",
        ));
    }

    let proxy_host = proxy
        .host_str()
        .ok_or_else(|| TlsError::proxy("https proxy has no host"))?;
    let tcp_stream = super::connect_to_proxy(connector, proxy).await?;

    let proxy_key = SessionCache::key(
        proxy_host,
        proxy.port_or_known_default().unwrap_or(443),
        None,
    );
    connector
        .do_tls_handshake(tcp_stream, proxy_host, &proxy_key, false)
        .await
        .map_err(TlsError::into_proxy)
}

async fn write_connect_and_validate<S>(
    stream: &mut S,
    host: &str,
    port: u16,
    proxy: &url::Url,
) -> Result<(), TlsError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let connect_req = if let Some(credentials) = proxy_basic_auth(proxy) {
        format!(
            "CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\nProxy-Authorization: {credentials}\r\n\r\n"
        )
    } else {
        format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n")
    };

    stream
        .write_all(connect_req.as_bytes())
        .await
        .map_err(TlsError::proxy_io)?;

    let mut response_buf = Vec::with_capacity(1024);
    let mut tmp = [0u8; 256];
    let end_idx = loop {
        let n = stream.read(&mut tmp).await.map_err(TlsError::proxy_io)?;
        if n == 0 {
            return Err(TlsError::proxy(
                "proxy closed connection before CONNECT response",
            ));
        }
        response_buf.extend_from_slice(&tmp[..n]);
        if response_buf.len() > 8192 {
            return Err(TlsError::proxy("proxy CONNECT response too large"));
        }
        if let Some(i) = response_buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };

    validate_connect_response(&response_buf, end_idx)
}

pub fn validate_connect_response(buf: &[u8], end_idx: usize) -> Result<(), TlsError> {
    if end_idx < 4 || end_idx > buf.len() {
        return Err(TlsError::proxy(format!(
            "proxy CONNECT response validator called with out-of-contract end_idx={end_idx} buf.len={}",
            buf.len()
        )));
    }

    let response = std::str::from_utf8(&buf[..end_idx])
        .map_err(|e| TlsError::proxy(format!("proxy CONNECT response is not UTF-8: {e}")))?;
    let mut lines = response.split("\r\n");
    let status_line = lines.next().unwrap_or("");

    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    let code = parts.next().unwrap_or("");
    if !matches!(version, "HTTP/1.1" | "HTTP/1.0") || code != "200" {
        let status = code.parse().ok();
        let detail = format!("proxy CONNECT failed: {status_line}");
        return Err(match status {
            Some(code) if TARGET_UNREACHABLE_STATUSES.contains(&code) => {
                TlsError::proxy_target(ProxyReply::HttpStatus(code), detail)
            }
            _ => TlsError::Proxy {
                status,
                detail,
                source: None,
            },
        });
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
                return Err(TlsError::proxy(format!(
                    "proxy CONNECT response contains forbidden framing header `{name}` \
                     (RFC 9110 §9.3.6)"
                )));
            }
        }
    }

    if end_idx < buf.len() {
        return Err(TlsError::proxy(
            "proxy CONNECT response carries trailing bytes after headers \
             (possible TLS-stream injection)",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests;
