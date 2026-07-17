//! SOCKS5 (RFC 1928 + 1929) tunnel establishment.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::tls::TlsStream;
use crate::tls::error::TlsError;

use crate::util::percent_decode;

/// Open a TLS-over-SOCKS5 tunnel through `proxy` and return the
/// wrapped TLS stream.
pub(crate) async fn connect<C: crate::tls::TlsHandshake>(
    connector: &C,
    host: &str,
    port: u16,
    proxy: &url::Url,
    include_alps: bool,
) -> Result<TlsStream, TlsError> {
    let auth = auth_request(proxy)?;
    let mut tcp_stream = super::connect_to_proxy(connector, proxy, 1080).await?;

    // Greeting: version 5, auth methods.
    if auth.is_some() {
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

    match (method_resp[1], auth.as_deref()) {
        // NO_AUTH is always offered and always acceptable.
        (0x00, _) => {}
        // USERNAME/PASSWORD — only valid if *we* offered it. RFC 1928 requires
        // the server to select from the methods the client sent; a server that
        // picks 0x02 when we only offered NO_AUTH is attempting an unsolicited
        // auth downgrade. Fall through to the rejection arm rather than sending
        // credentials (or empty ones) we never advertised.
        (0x02, Some(auth)) => authenticate(&mut tcp_stream, auth).await?,
        (0xFF, _) => {
            return Err(TlsError::Profile(
                "socks5: no acceptable auth method".into(),
            ));
        }
        (other, _) => {
            return Err(TlsError::Profile(format!(
                "socks5: unsupported auth method 0x{other:02x}"
            )));
        }
    }

    send_connect(&mut tcp_stream, host, port).await?;

    connector
        .do_tls_handshake(tcp_stream, host, include_alps)
        .await
}

fn auth_request(proxy: &url::Url) -> Result<Option<Vec<u8>>, TlsError> {
    let username = percent_decode(proxy.username());
    let password = proxy.password().map(percent_decode);

    if username.is_empty() && password.is_none() {
        return Ok(None);
    }

    let Some(password) = password else {
        return Err(TlsError::Profile(
            "socks5: username and password must both be present".into(),
        ));
    };
    if username.is_empty() || password.is_empty() {
        return Err(TlsError::Profile(
            "socks5: username and password must both be non-empty".into(),
        ));
    }
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
    Ok(Some(auth_req))
}

async fn authenticate(tcp_stream: &mut TcpStream, auth: &[u8]) -> Result<(), TlsError> {
    tcp_stream
        .write_all(auth)
        .await
        .map_err(TlsError::TcpConnect)?;

    let mut auth_resp = [0u8; 2];
    tcp_stream
        .read_exact(&mut auth_resp)
        .await
        .map_err(TlsError::TcpConnect)?;
    // RFC 1929 §2: VER must be 0x01. A wrong version byte means the
    // peer is not speaking the sub-negotiation protocol — treat any
    // status it carries as garbage rather than trusting byte 1 alone.
    if auth_resp[0] != 0x01 {
        return Err(TlsError::Profile(format!(
            "socks5: invalid auth sub-negotiation version 0x{:02x}",
            auth_resp[0]
        )));
    }
    if auth_resp[1] != 0x00 {
        return Err(TlsError::Profile("socks5: authentication failed".into()));
    }
    Ok(())
}

async fn send_connect(tcp_stream: &mut TcpStream, host: &str, port: u16) -> Result<(), TlsError> {
    let host_bytes = host.as_bytes();
    // RFC 1928 §5: domain-name address type carries a one-byte length.
    // Without this guard the `as u8` below silently truncates and the
    // proxy misparses the request.
    if host_bytes.len() > 255 {
        return Err(TlsError::Profile(format!(
            "socks5: hostname too long ({} bytes, max 255)",
            host_bytes.len()
        )));
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::net::TcpListener;

    async fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (client, server) = tokio::join!(TcpStream::connect(addr), listener.accept());
        (client.unwrap(), server.unwrap().0)
    }

    // RFC 1929 §2: the auth sub-negotiation response is VER STATUS and
    // VER MUST be 0x01. A proxy (or in-path injector) replying with a
    // garbage version byte and a success status must not be accepted.
    #[tokio::test]
    async fn authenticate_rejects_wrong_subnegotiation_version() {
        let (mut client, mut server) = pair().await;
        let proxy: url::Url = "socks5://user:pass@127.0.0.1:1080".parse().unwrap();
        let auth = auth_request(&proxy).unwrap().unwrap();
        let server_task = tokio::spawn(async move {
            let mut buf = vec![0u8; 64];
            let _ = server.read(&mut buf).await.unwrap();
            server.write_all(&[0x05, 0x00]).await.unwrap();
            server // keep the socket open until the client is done
        });
        let res = authenticate(&mut client, &auth).await;
        assert!(
            res.is_err(),
            "malformed auth VER byte must be rejected, got {res:?}"
        );
        let _ = server_task.await;
    }

    #[test]
    fn auth_request_encodes_rfc_1929_credentials() {
        let proxy: url::Url = "socks5://u%73er:p%40ss@127.0.0.1:1080".parse().unwrap();

        assert_eq!(
            auth_request(&proxy).unwrap(),
            Some(vec![
                0x01, 0x04, b'u', b's', b'e', b'r', 0x04, b'p', b'@', b's', b's'
            ])
        );
    }

    #[test]
    fn auth_request_leaves_no_auth_proxies_unmodified() {
        let proxy: url::Url = "socks5://127.0.0.1:1080".parse().unwrap();

        assert_eq!(auth_request(&proxy).unwrap(), None);
    }

    #[test]
    fn auth_request_accepts_255_byte_credentials() {
        let user = "u".repeat(255);
        let password = "p".repeat(255);
        let proxy: url::Url = format!("socks5://{user}:{password}@127.0.0.1:1080")
            .parse()
            .unwrap();

        let auth = auth_request(&proxy).unwrap().unwrap();
        assert_eq!(auth.len(), 513);
        assert_eq!(&auth[..2], &[0x01, 255]);
        assert_eq!(auth[257], 255);
    }

    #[test]
    fn auth_request_rejects_incomplete_empty_and_oversized_credentials() {
        for proxy in [
            "socks5://user@127.0.0.1:1080",
            "socks5://:pass@127.0.0.1:1080",
            "socks5://user:@127.0.0.1:1080",
            &format!("socks5://{}:pass@127.0.0.1:1080", "u".repeat(256)),
            &format!("socks5://user:{}@127.0.0.1:1080", "p".repeat(256)),
        ] {
            let proxy: url::Url = proxy.parse().unwrap();
            assert!(auth_request(&proxy).is_err(), "{proxy}");
        }
    }

    // RFC 1928 §5: the domain-name address type carries a one-byte
    // length, so hostnames past 255 bytes cannot be encoded. The old
    // code truncated the length with `as u8` and sent a malformed
    // CONNECT that the proxy misparses.
    #[tokio::test]
    async fn send_connect_rejects_hostname_longer_than_255_bytes() {
        let (mut client, _server) = pair().await;
        let long_host = "a".repeat(256);
        let res = tokio::time::timeout(
            Duration::from_secs(1),
            send_connect(&mut client, &long_host, 443),
        )
        .await;
        assert!(
            matches!(res, Ok(Err(_))),
            "256-byte hostname must error immediately, got {res:?}"
        );
    }
}
