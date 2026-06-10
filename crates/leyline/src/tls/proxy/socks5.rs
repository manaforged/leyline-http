//! SOCKS5 (RFC 1928 + 1929) tunnel establishment.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::tls::connector::FingerprintConnector;
use crate::tls::error::TlsError;
use crate::tls::TlsStream;

use crate::util::percent_decode;

/// Open a TLS-over-SOCKS5 tunnel through `proxy` and return the
/// wrapped TLS stream.
pub(crate) async fn connect(
    connector: &FingerprintConnector,
    host: &str,
    port: u16,
    proxy: &url::Url,
    include_alps: bool,
) -> Result<TlsStream, TlsError> {
    let proxy_host = proxy
        .host_str()
        .ok_or_else(|| TlsError::Profile("socks5 proxy has no host".into()))?;
    // `port_or_known_default()` is defensive — `url` only knows http/https/ws/wss
    // defaults, so socks5 still falls through to `unwrap_or(1080)`.
    let proxy_port = proxy.port_or_known_default().unwrap_or(1080);

    let proxy_addr = format!("{proxy_host}:{proxy_port}");
    let mut tcp_stream = TcpStream::connect(&proxy_addr)
        .await
        .map_err(TlsError::TcpConnect)?;

    let has_auth = !proxy.username().is_empty();

    // Greeting: version 5, auth methods.
    if has_auth {
        // Offer NO_AUTH (0x00) and USERNAME/PASSWORD (0x02).
        tcp_stream
            .write_all(&[0x05, 0x02, 0x00, 0x02])
            .await
            .map_err(TlsError::TcpConnect)?;
    } else {
        tcp_stream
            .write_all(&[0x05, 0x01, 0x00])
            .await
            .map_err(TlsError::TcpConnect)?;
    }

    let mut method_resp = [0u8; 2];
    tcp_stream
        .read_exact(&mut method_resp)
        .await
        .map_err(TlsError::TcpConnect)?;

    if method_resp[0] != 0x05 {
        return Err(TlsError::Profile(
            "socks5: invalid version in response".into(),
        ));
    }

    match method_resp[1] {
        0x00 => {}
        0x02 => authenticate(&mut tcp_stream, proxy).await?,
        0xFF => {
            return Err(TlsError::Profile(
                "socks5: no acceptable auth method".into(),
            ))
        }
        other => {
            return Err(TlsError::Profile(format!(
                "socks5: unsupported auth method 0x{other:02x}"
            )))
        }
    }

    send_connect(&mut tcp_stream, host, port).await?;

    connector
        .tls_handshake(tcp_stream, host, include_alps)
        .await
}

async fn authenticate(tcp_stream: &mut TcpStream, proxy: &url::Url) -> Result<(), TlsError> {
    let username = percent_decode(proxy.username());
    let password = proxy.password().map(percent_decode).unwrap_or_default();
    if username.len() > 255 || password.len() > 255 {
        return Err(TlsError::Profile(
            "socks5: username or password exceeds 255 bytes".into(),
        ));
    }
    let mut auth_req = Vec::with_capacity(3 + username.len() + password.len());
    auth_req.push(0x01); // Sub-negotiation version.
    auth_req.push(username.len() as u8);
    auth_req.extend_from_slice(username.as_bytes());
    auth_req.push(password.len() as u8);
    auth_req.extend_from_slice(password.as_bytes());
    tcp_stream
        .write_all(&auth_req)
        .await
        .map_err(TlsError::TcpConnect)?;

    let mut auth_resp = [0u8; 2];
    tcp_stream
        .read_exact(&mut auth_resp)
        .await
        .map_err(TlsError::TcpConnect)?;
    if auth_resp[1] != 0x00 {
        return Err(TlsError::Profile("socks5: authentication failed".into()));
    }
    Ok(())
}

async fn send_connect(tcp_stream: &mut TcpStream, host: &str, port: u16) -> Result<(), TlsError> {
    let host_bytes = host.as_bytes();
    let mut connect_req = Vec::with_capacity(7 + host_bytes.len());
    connect_req.push(0x05); // Version.
    connect_req.push(0x01); // CONNECT command.
    connect_req.push(0x00); // Reserved.
    connect_req.push(0x03); // Domain name address type.
    connect_req.push(host_bytes.len() as u8);
    connect_req.extend_from_slice(host_bytes);
    connect_req.push((port >> 8) as u8);
    connect_req.push(port as u8);
    tcp_stream
        .write_all(&connect_req)
        .await
        .map_err(TlsError::TcpConnect)?;

    let mut resp_buf = [0u8; 4];
    tcp_stream
        .read_exact(&mut resp_buf)
        .await
        .map_err(TlsError::TcpConnect)?;

    if resp_buf[0] != 0x05 {
        return Err(TlsError::Profile(
            "socks5: invalid CONNECT response version".into(),
        ));
    }
    if resp_buf[1] != 0x00 {
        let reason = match resp_buf[1] {
            0x01 => "general failure",
            0x02 => "connection not allowed",
            0x03 => "network unreachable",
            0x04 => "host unreachable",
            0x05 => "connection refused",
            0x06 => "TTL expired",
            0x07 => "command not supported",
            0x08 => "address type not supported",
            _ => "unknown error",
        };
        return Err(TlsError::Profile(format!(
            "socks5: CONNECT failed: {reason}"
        )));
    }

    // Skip the bind address. Address type is at resp_buf[3].
    match resp_buf[3] {
        0x01 => {
            // IPv4: 4 bytes + 2 port bytes.
            let mut skip = [0u8; 6];
            tcp_stream
                .read_exact(&mut skip)
                .await
                .map_err(TlsError::TcpConnect)?;
        }
        0x03 => {
            // Domain: 1 byte length + N bytes + 2 port bytes.
            let mut len_buf = [0u8; 1];
            tcp_stream
                .read_exact(&mut len_buf)
                .await
                .map_err(TlsError::TcpConnect)?;
            let skip_len = len_buf[0] as usize + 2;
            let mut skip = vec![0u8; skip_len];
            tcp_stream
                .read_exact(&mut skip)
                .await
                .map_err(TlsError::TcpConnect)?;
        }
        0x04 => {
            // IPv6: 16 bytes + 2 port bytes.
            let mut skip = [0u8; 18];
            tcp_stream
                .read_exact(&mut skip)
                .await
                .map_err(TlsError::TcpConnect)?;
        }
        other => {
            return Err(TlsError::Profile(format!(
                "socks5: unknown address type 0x{other:02x}"
            )));
        }
    }

    Ok(())
}
