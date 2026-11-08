use http::header::{CONTENT_LENGTH, TRANSFER_ENCODING};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::error::Error;
use crate::net::{Dialer, NetError, ProxyReply};
use crate::util::proxy_basic_auth;
use crate::wire::h1::{self, MAX_H1_HEADER_BYTES, Sink};

const TARGET_UNREACHABLE_STATUSES: [u16; 2] = [502, 504];

const FRAMING_HEADERS: [http::HeaderName; 2] = [CONTENT_LENGTH, TRANSFER_ENCODING];

pub(crate) async fn tunnel(
    dialer: &Dialer,
    host: &str,
    port: u16,
    proxy: &url::Url,
) -> Result<TcpStream, NetError> {
    let mut tcp_stream = super::connect_to_proxy(dialer, proxy).await?;
    write_connect_and_validate(&mut tcp_stream, host, port, proxy).await?;
    Ok(tcp_stream)
}

pub(crate) async fn write_connect_and_validate<S>(
    stream: &mut S,
    host: &str,
    port: u16,
    proxy: &url::Url,
) -> Result<(), NetError>
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
        .map_err(NetError::proxy_io)?;

    let mut response_buf = Vec::with_capacity(1024);
    let mut tmp = [0u8; 256];
    let end_idx = loop {
        let n = stream.read(&mut tmp).await.map_err(NetError::proxy_io)?;
        if n == 0 {
            return Err(NetError::proxy(
                "proxy closed connection before CONNECT response",
            ));
        }
        response_buf.extend_from_slice(&tmp[..n]);
        if response_buf.len() > MAX_H1_HEADER_BYTES {
            return Err(NetError::proxy("proxy CONNECT response too large"));
        }
        if let Some(i) = h1::find_header_end(&response_buf) {
            break i + 4;
        }
    };

    validate_connect_response(&response_buf, end_idx)
}

/// Checks an HTTP CONNECT reply head ending at `end_idx`.
pub fn validate_connect_response(buf: &[u8], end_idx: usize) -> Result<(), NetError> {
    if end_idx < 4 || end_idx > buf.len() {
        return Err(NetError::proxy(format!(
            "proxy CONNECT response validator called with out-of-contract end_idx={end_idx} buf.len={}",
            buf.len()
        )));
    }

    let head = &buf[..end_idx];
    let response = std::str::from_utf8(head)
        .map_err(|e| NetError::proxy(format!("proxy CONNECT response is not UTF-8: {e}")))?;
    let status_line = response.split("\r\n").next().unwrap_or("");
    let status = h1::status_line(status_line.as_bytes())
        .ok()
        .map(|(code, _)| code);
    if status != Some(200) {
        let detail = format!("proxy CONNECT failed: {status_line}");
        return Err(match status {
            Some(code) if TARGET_UNREACHABLE_STATUSES.contains(&code) => {
                NetError::proxy_target(ProxyReply::HttpStatus(code), detail)
            }
            _ => NetError::Proxy {
                status,
                detail,
                source: None,
            },
        });
    }

    let mut framing = Framing::default();
    h1::parse(head, &mut framing).map_err(|e| {
        NetError::proxy(format!(
            "proxy CONNECT response is malformed: {}",
            e.message().unwrap_or_default()
        ))
    })?;
    if let Some(name) = framing.found {
        return Err(NetError::proxy(format!(
            "proxy CONNECT response contains forbidden framing header `{}` \
             (RFC 9110 §9.3.6)",
            String::from_utf8_lossy(name)
        )));
    }

    if end_idx < buf.len() {
        return Err(NetError::proxy(
            "proxy CONNECT response carries trailing bytes after headers \
             (possible TLS-stream injection)",
        ));
    }

    Ok(())
}

#[derive(Default)]
struct Framing<'a> {
    found: Option<&'a [u8]>,
}

impl<'a> Sink<'a> for Framing<'a> {
    fn field(&mut self, name: &'a [u8], _value: &'a [u8]) -> Result<(), Error> {
        let framing = FRAMING_HEADERS
            .iter()
            .any(|header| name.eq_ignore_ascii_case(header.as_str().as_bytes()));
        if framing && self.found.is_none() {
            self.found = Some(name);
        }
        Ok(())
    }

    fn fold(&mut self, _line: &'a [u8]) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
