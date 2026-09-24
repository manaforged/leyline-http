use std::net::{IpAddr, SocketAddr};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::tls::error::TlsError;
use crate::tls::{SessionCache, TlsStream};

use crate::util::percent_decode;

const CMD_CONNECT: u8 = 0x01;
const CMD_UDP_ASSOCIATE: u8 = 0x03;

pub(crate) async fn connect<C: crate::tls::TlsHandshake>(
    connector: &C,
    host: &str,
    port: u16,
    proxy: &url::Url,
    include_alps: bool,
) -> Result<TlsStream, TlsError> {
    let mut tcp_stream = open_control(connector, proxy).await?;
    send_connect(&mut tcp_stream, host, port).await?;

    let session_key = SessionCache::key(host, port, Some(proxy));
    connector
        .do_tls_handshake(tcp_stream, host, &session_key, include_alps)
        .await
}

async fn send_connect(tcp_stream: &mut TcpStream, host: &str, port: u16) -> Result<(), TlsError> {
    request(tcp_stream, CMD_CONNECT, &encode_addr(host, port)?)
        .await
        .map(drop)
}

async fn open_control<C: crate::tls::TlsHandshake>(
    connector: &C,
    proxy: &url::Url,
) -> Result<TcpStream, TlsError> {
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
    Ok(tcp_stream)
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

pub(crate) fn encode_addr(host: &str, port: u16) -> Result<Vec<u8>, TlsError> {
    let bare = crate::util::bare_host(host);
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return Ok(encode_socket_addr(SocketAddr::new(ip, port)));
    }
    let host_bytes = host.as_bytes();
    let len = u8::try_from(host_bytes.len()).map_err(|_| {
        TlsError::proxy(format!(
            "socks5: hostname too long ({} bytes, max 255)",
            host_bytes.len()
        ))
    })?;
    let mut out = Vec::with_capacity(4 + host_bytes.len());
    out.push(0x03);
    out.push(len);
    out.extend_from_slice(host_bytes);
    out.extend_from_slice(&port.to_be_bytes());
    Ok(out)
}

fn encode_socket_addr(addr: SocketAddr) -> Vec<u8> {
    let mut out = Vec::with_capacity(19);
    match addr.ip() {
        IpAddr::V4(ip) => {
            out.push(0x01);
            out.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            out.push(0x04);
            out.extend_from_slice(&ip.octets());
        }
    }
    out.extend_from_slice(&addr.port().to_be_bytes());
    out
}

pub(crate) fn addr_len(atyp: u8, next: Option<u8>) -> Result<usize, TlsError> {
    match (atyp, next) {
        (0x01, _) => Ok(4 + 2),
        (0x04, _) => Ok(16 + 2),
        (0x03, Some(len)) => Ok(1 + len as usize + 2),
        (0x03, None) => Err(TlsError::proxy("socks5: truncated domain address")),
        (other, _) => Err(TlsError::proxy(format!(
            "socks5: unknown address type 0x{other:02x}"
        ))),
    }
}

async fn request(
    tcp_stream: &mut TcpStream,
    cmd: u8,
    addr: &[u8],
) -> Result<(u8, Vec<u8>), TlsError> {
    let mut req = Vec::with_capacity(3 + addr.len());
    req.extend_from_slice(&[0x05, cmd, 0x00]);
    req.extend_from_slice(addr);
    tcp_stream
        .write_all(&req)
        .await
        .map_err(TlsError::proxy_io)?;

    let mut resp_buf = [0u8; 4];
    tcp_stream
        .read_exact(&mut resp_buf)
        .await
        .map_err(TlsError::proxy_io)?;

    if resp_buf[0] != 0x05 {
        return Err(TlsError::proxy("socks5: invalid reply version"));
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
        let name = if cmd == CMD_UDP_ASSOCIATE {
            "UDP ASSOCIATE"
        } else {
            "CONNECT"
        };
        return Err(TlsError::proxy(format!("socks5: {name} failed: {reason}")));
    }

    let atyp = resp_buf[3];
    let mut first = [0u8; 1];
    let domain_len = if atyp == 0x03 {
        tcp_stream
            .read_exact(&mut first)
            .await
            .map_err(TlsError::proxy_io)?;
        Some(first[0])
    } else {
        None
    };
    let len = addr_len(atyp, domain_len)? - usize::from(domain_len.is_some());
    let mut body = vec![0u8; len];
    tcp_stream
        .read_exact(&mut body)
        .await
        .map_err(TlsError::proxy_io)?;
    Ok((atyp, body))
}

#[cfg(feature = "http3")]
pub(crate) mod udp;

#[cfg(test)]
mod tests;
