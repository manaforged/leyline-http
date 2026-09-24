use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::tls::error::TlsError;
use crate::tls::{SessionCache, TlsStream};

use crate::util::percent_decode;

pub(crate) async fn connect<C: crate::tls::TlsHandshake>(
    connector: &C,
    host: &str,
    port: u16,
    proxy: &url::Url,
    include_alps: bool,
) -> Result<TlsStream, TlsError> {
    let auth = auth_request(proxy)?;
    let mut tcp_stream = super::connect_to_proxy(connector, proxy, 1080).await?;

    if auth.is_some() {
        tcp_stream
            .write_all(&[0x05, 0x02, 0x00, 0x02])
            .await
            .map_err(TlsError::proxy_io)?;
    } else {
        tcp_stream
            .write_all(&[0x05, 0x01, 0x00])
            .await
            .map_err(TlsError::proxy_io)?;
    }

    let mut method_resp = [0u8; 2];
    tcp_stream
        .read_exact(&mut method_resp)
        .await
        .map_err(TlsError::proxy_io)?;

    if method_resp[0] != 0x05 {
        return Err(TlsError::proxy("socks5: invalid version in response"));
    }

    match (method_resp[1], auth.as_deref()) {
        (0x00, _) => {}
        (0x02, Some(auth)) => authenticate(&mut tcp_stream, auth).await?,
        (0xFF, _) => {
            return Err(TlsError::proxy("socks5: no acceptable auth method"));
        }
        (other, _) => {
            return Err(TlsError::proxy(format!(
                "socks5: unsupported auth method 0x{other:02x}"
            )));
        }
    }

    send_connect(&mut tcp_stream, host, port).await?;

    let session_key = SessionCache::key(host, port, Some(proxy));
    connector
        .do_tls_handshake(tcp_stream, host, &session_key, include_alps)
        .await
}

fn auth_request(proxy: &url::Url) -> Result<Option<Vec<u8>>, TlsError> {
    let username = percent_decode(proxy.username());
    let password = proxy.password().map(percent_decode);

    if username.is_empty() && password.is_none() {
        return Ok(None);
    }

    let Some(password) = password else {
        return Err(TlsError::proxy(
            "socks5: username and password must both be present",
        ));
    };
    if username.is_empty() || password.is_empty() {
        return Err(TlsError::proxy(
            "socks5: username and password must both be non-empty",
        ));
    }
    if username.len() > 255 || password.len() > 255 {
        return Err(TlsError::proxy(
            "socks5: username or password exceeds 255 bytes",
        ));
    }
    let mut auth_req = Vec::with_capacity(3 + username.len() + password.len());
    auth_req.push(0x01);
    auth_req.push(username.len() as u8);
    auth_req.extend_from_slice(username.as_bytes());
    auth_req.push(password.len() as u8);
    auth_req.extend_from_slice(password.as_bytes());
    Ok(Some(auth_req))
}

async fn authenticate(tcp_stream: &mut TcpStream, auth: &[u8]) -> Result<(), TlsError> {
    tcp_stream
        .write_all(auth)
        .await
        .map_err(TlsError::proxy_io)?;

    let mut auth_resp = [0u8; 2];
    tcp_stream
        .read_exact(&mut auth_resp)
        .await
        .map_err(TlsError::proxy_io)?;
    if auth_resp[0] != 0x01 {
        return Err(TlsError::proxy(format!(
            "socks5: invalid auth sub-negotiation version 0x{:02x}",
            auth_resp[0]
        )));
    }
    if auth_resp[1] != 0x00 {
        return Err(TlsError::proxy("socks5: authentication failed"));
    }
    Ok(())
}

async fn send_connect(tcp_stream: &mut TcpStream, host: &str, port: u16) -> Result<(), TlsError> {
    let mut connect_req = vec![0x05, 0x01, 0x00];
    match crate::util::bare_host(host).parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            connect_req.push(0x01);
            connect_req.extend_from_slice(&ip.octets());
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            connect_req.push(0x04);
            connect_req.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            let host_bytes = host.as_bytes();
            let len = u8::try_from(host_bytes.len()).map_err(|_| {
                TlsError::proxy(format!(
                    "socks5: hostname too long ({} bytes, max 255)",
                    host_bytes.len()
                ))
            })?;
            connect_req.push(0x03);
            connect_req.push(len);
            connect_req.extend_from_slice(host_bytes);
        }
    }
    connect_req.extend_from_slice(&port.to_be_bytes());
    tcp_stream
        .write_all(&connect_req)
        .await
        .map_err(TlsError::proxy_io)?;

    let mut resp_buf = [0u8; 4];
    tcp_stream
        .read_exact(&mut resp_buf)
        .await
        .map_err(TlsError::proxy_io)?;

    if resp_buf[0] != 0x05 {
        return Err(TlsError::proxy("socks5: invalid CONNECT response version"));
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
        return Err(TlsError::proxy(format!("socks5: CONNECT failed: {reason}")));
    }

    match resp_buf[3] {
        0x01 => {
            let mut skip = [0u8; 6];
            tcp_stream
                .read_exact(&mut skip)
                .await
                .map_err(TlsError::proxy_io)?;
        }
        0x03 => {
            let mut len_buf = [0u8; 1];
            tcp_stream
                .read_exact(&mut len_buf)
                .await
                .map_err(TlsError::proxy_io)?;
            let skip_len = len_buf[0] as usize + 2;
            let mut skip = vec![0u8; skip_len];
            tcp_stream
                .read_exact(&mut skip)
                .await
                .map_err(TlsError::proxy_io)?;
        }
        0x04 => {
            let mut skip = [0u8; 18];
            tcp_stream
                .read_exact(&mut skip)
                .await
                .map_err(TlsError::proxy_io)?;
        }
        other => {
            return Err(TlsError::proxy(format!(
                "socks5: unknown address type 0x{other:02x}"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
