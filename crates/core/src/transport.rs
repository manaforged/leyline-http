//! Transport layer — connects TLS/plain TCP and sends requests.
//!
//! HTTPS defaults to our own leyline-h2 implementation with connection pooling.
//! Plain `http://` and explicit H1 policy use a small HTTP/1.1 transport.

use std::borrow::Cow;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::StreamExt;
use leyline_h2::config::H2Config;
use leyline_h2::connection::PseudoHeaders;
use leyline_pool::Pool;
use leyline_tls::FingerprintConnector;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::body::Body;
use crate::body_stream::BodyStream;
use crate::error::{Error, Result};
use crate::response::HttpVersion;

const MAX_H1_HEADER_BYTES: usize = 64 * 1024;
const MAX_H1_BODY_BYTES: usize = 100 * 1024 * 1024;

/// Body shape returned by a transport. Either fully buffered, or a
/// receiver the caller drains via `BodyStream`.
pub(crate) enum TransportBody {
    Buffered(Vec<u8>),
    Streaming(BodyStream),
}

/// Response returned by a transport.
pub(crate) struct TransportResponse {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: TransportBody,
    pub(crate) final_url: String,
    pub(crate) version: HttpVersion,
    pub(crate) tls_alpn: Option<String>,
    pub(crate) peer_cert_der: Option<Vec<u8>>,
    pub(crate) tls_version: Option<String>,
    pub(crate) tls_cipher: Option<String>,
}

/// Send an HTTP request with browser-compatible defaults.
#[tracing::instrument(
    name = "transport.auto",
    level = "debug",
    skip_all,
    fields(
        http.method = method,
        http.scheme = url.scheme(),
        http.host = url.host_str().unwrap_or(""),
        proxied = proxy.is_some(),
    )
)]
pub(crate) async fn send_request_auto(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: Body,
    proxy: Option<&str>,
    stream_response: bool,
) -> Result<TransportResponse> {
    if url.scheme() == "http" {
        return send_request_h1(connector, method, url, headers, body, proxy, stream_response)
            .await;
    }

    // To support the H1 fallback on ALPN mismatch we need to retain the
    // body. Streaming bodies are one-shot, so eagerly materialise them.
    // Callers who want hard streaming over H2 should pin `.http2()`.
    let (h2_body, fallback_buf): (Body, Option<Bytes>) = if body.is_stream() {
        let buf = materialise_stream_body(body).await?;
        (Body::from(buf.clone()), Some(buf))
    } else {
        match body {
            Body::Empty => (Body::Empty, None),
            Body::Bytes(b) => (Body::Bytes(b.clone()), Some(b)),
            Body::Stream { .. } => unreachable!("stream branch handled above"),
        }
    };

    match send_request_h2(
        pool,
        connector,
        h2_config,
        method,
        url,
        headers.clone(),
        h2_body,
        proxy,
        stream_response,
    )
    .await
    {
        Ok(resp) => Ok(resp),
        Err(e) if is_h2_alpn_mismatch(&e) => {
            tracing::debug!(error = %e, "H2 ALPN mismatch, falling back to HTTP/1.1");
            let fallback_body = match fallback_buf {
                Some(buf) => Body::from(buf),
                None => Body::Empty,
            };
            send_request_h1(
                connector,
                method,
                url,
                headers,
                fallback_body,
                proxy,
                stream_response,
            )
            .await
        }
        Err(e) => Err(e),
    }
}

/// Drain a streaming body into a single `Bytes` buffer. Used when the
/// transport can't accept streams (H3) or needs to retain the body for
/// a fallback retry.
async fn materialise_stream_body(body: Body) -> Result<Bytes> {
    match body {
        Body::Empty => Ok(Bytes::new()),
        Body::Bytes(b) => Ok(b),
        Body::Stream { mut stream, .. } => {
            let mut buf: Vec<u8> = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(Error::Io)?;
                buf.extend_from_slice(&chunk);
                if buf.len() > MAX_H1_BODY_BYTES {
                    return Err(Error::Http(format!(
                        "streaming request body exceeded {MAX_H1_BODY_BYTES} bytes"
                    )));
                }
            }
            Ok(Bytes::from(buf))
        }
    }
}

/// Send an HTTP request, reusing pooled H2 connections when available.
#[tracing::instrument(
    name = "transport.h2",
    level = "debug",
    skip_all,
    fields(http.method = method, http.host = url.host_str().unwrap_or(""))
)]
pub(crate) async fn send_request_h2(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: Body,
    proxy: Option<&str>,
    stream_response: bool,
) -> Result<TransportResponse> {
    if url.scheme() != "https" {
        return Err(Error::Config("HTTP/2 requires an https:// URL".into()));
    }

    let host = url
        .host_str()
        .ok_or_else(|| Error::Config("no host in URL".into()))?;
    let port = url.port_or_known_default().unwrap_or(443);

    // Build pseudo-headers.
    let path = url.path();
    let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
    let pseudo = PseudoHeaders {
        method: method.to_string(),
        scheme: url.scheme().to_string(),
        authority: {
            let is_default_port =
                (url.scheme() == "https" && port == 443) || (url.scheme() == "http" && port == 80);
            if is_default_port {
                host.to_string()
            } else {
                format!("{host}:{port}")
            }
        },
        path: format!("{path}{query}"),
        protocol: None,
    };

    // Translate Body → h2 request body representation.
    let h2_req_body = body_to_h2_request(body);

    // Send via pool (reuses connection or creates new one).
    let (resp, tls) = leyline_pool::send_request(
        pool,
        connector,
        h2_config,
        pseudo,
        headers,
        h2_req_body,
        proxy,
        stream_response,
    )
    .await
    .map_err(|e| Error::Http(e))?;

    let transport_body = match resp.body {
        leyline_h2::client::ResponseBody::Buffered(b) => TransportBody::Buffered(b),
        leyline_h2::client::ResponseBody::Streaming(rx) => {
            TransportBody::Streaming(BodyStream::new(rx))
        }
    };

    Ok(TransportResponse {
        status: resp.status,
        headers: resp.headers,
        body: transport_body,
        final_url: url.to_string(),
        version: HttpVersion::Http2,
        tls_alpn: Some("h2".to_string()),
        peer_cert_der: tls.peer_cert_der,
        tls_version: tls.version,
        tls_cipher: tls.cipher,
    })
}

/// Translate a [`Body`] into the h2-crate request body shape.
fn body_to_h2_request(body: Body) -> leyline_h2::client::RequestBody {
    match body {
        Body::Empty => leyline_h2::client::RequestBody::None,
        Body::Bytes(b) => leyline_h2::client::RequestBody::Buffered(b),
        Body::Stream {
            stream,
            length_hint,
        } => leyline_h2::client::RequestBody::Streaming {
            stream,
            length_hint,
        },
    }
}

/// Send an HTTP/1.1 request over plaintext TCP or TLS.
#[tracing::instrument(
    name = "transport.h1",
    level = "debug",
    skip_all,
    fields(
        http.method = method,
        http.scheme = url.scheme(),
        http.host = url.host_str().unwrap_or(""),
    )
)]
pub(crate) async fn send_request_h1(
    connector: &FingerprintConnector,
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: Body,
    proxy: Option<&str>,
    _stream_response: bool,
) -> Result<TransportResponse> {
    let host = url
        .host_str()
        .ok_or_else(|| Error::Config("no host in URL".into()))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| Error::Config(format!("no default port for scheme {}", url.scheme())))?;

    // Streaming responses over H1 are not yet wired through — the body
    // is buffered, then wrapped in a single-chunk stream by the core on
    // `into_stream()`. Streaming requests are handled here directly.
    match url.scheme() {
        "https" => {
            let tls_stream = connector
                .connect_h1(host, port, proxy)
                .await
                .map_err(Error::Tls)?;
            let alpn = tls_stream
                .alpn
                .as_ref()
                .map(|p| String::from_utf8_lossy(p).to_string());
            let peer_cert_der = tls_stream.peer_cert_der;
            let tls_version = tls_stream.tls_version;
            let tls_cipher = tls_stream.tls_cipher;
            let mut resp = send_h1_on_stream(
                tls_stream.stream,
                method,
                url,
                headers,
                body,
                RequestTarget::OriginForm,
            )
            .await?;
            resp.tls_alpn = alpn;
            resp.peer_cert_der = peer_cert_der;
            resp.tls_version = tls_version;
            resp.tls_cipher = tls_cipher;
            Ok(resp)
        }
        "http" => {
            if let Some(proxy_url) = proxy {
                let proxy = url::Url::parse(proxy_url)
                    .map_err(|e| Error::Config(format!("invalid proxy URL: {e}")))?;
                if proxy.scheme() != "http" {
                    return Err(Error::Config(
                        "plaintext HTTP currently supports http:// proxies only".into(),
                    ));
                }
                let proxy_host = proxy
                    .host_str()
                    .ok_or_else(|| Error::Config("proxy has no host".into()))?;
                let proxy_port = proxy.port().unwrap_or(8080);
                let stream = tokio::net::TcpStream::connect((proxy_host, proxy_port)).await?;
                let mut headers = headers;
                if let Some(password) = proxy.password() {
                    let credentials = base64_encode(&format!(
                        "{}:{}",
                        percent_decode(proxy.username()),
                        percent_decode(password)
                    ));
                    headers.push(("Proxy-Authorization".into(), format!("Basic {credentials}")));
                }
                send_h1_on_stream(
                    stream,
                    method,
                    url,
                    headers,
                    body,
                    RequestTarget::AbsoluteForm,
                )
                .await
            } else {
                let stream = tokio::net::TcpStream::connect((host, port)).await?;
                send_h1_on_stream(
                    stream,
                    method,
                    url,
                    headers,
                    body,
                    RequestTarget::OriginForm,
                )
                .await
            }
        }
        other => Err(Error::Config(format!(
            "unsupported URL scheme for HTTP/1.1: {other}"
        ))),
    }
}

/// Send an HTTP/3 request over QUIC.
#[tracing::instrument(
    name = "transport.h3",
    level = "debug",
    skip_all,
    fields(http.method = method, http.host = url.host_str().unwrap_or(""))
)]
pub(crate) async fn send_request_h3(
    h3_config: &leyline_quic::H3Config,
    profile: &leyline_profile::BrowserProfile,
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: Body,
    stream_response: bool,
) -> Result<TransportResponse> {
    // H3 streaming (request or response) is deferred — quiche-level
    // pump/pull plumbing is a separate piece of work. Reject the
    // request so callers see a clear error, not silent buffering.
    if body.is_stream() {
        return Err(Error::Config(
            "HTTP/3 streaming request bodies are not yet implemented; use .http2() or buffer \
             the body before sending"
                .into(),
        ));
    }
    if stream_response {
        return Err(Error::Config(
            "HTTP/3 streaming response bodies are not yet implemented; use .http2() or drop \
             .stream()"
                .into(),
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| Error::Config("no host in URL".into()))?;
    let port = url.port_or_known_default().unwrap_or(443);
    let path = url.path();
    let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
    let full_path = format!("{path}{query}");

    let body_bytes = match body {
        Body::Empty => None,
        Body::Bytes(b) => Some(b),
        Body::Stream { .. } => unreachable!("rejected above"),
    };

    let resp = leyline_quic::H3Connection::request(
        h3_config, profile, method, host, port, &full_path, headers, body_bytes,
    )
    .await
    .map_err(|e| Error::Http(e))?;

    Ok(TransportResponse {
        status: resp.status,
        headers: resp.headers,
        body: TransportBody::Buffered(resp.body),
        final_url: url.to_string(),
        version: HttpVersion::Http3,
        tls_alpn: Some("h3".to_string()),
        // H3/QUIC peer-cert extraction and TLS details are a separate
        // piece of work — the quiche path doesn't currently expose the
        // handshake result through the connection handle.
        peer_cert_der: None,
        tls_version: None,
        tls_cipher: None,
    })
}

#[derive(Clone, Copy)]
enum RequestTarget {
    OriginForm,
    AbsoluteForm,
}

async fn send_h1_on_stream<S>(
    mut stream: S,
    method: &str,
    url: &url::Url,
    mut headers: Vec<(String, String)>,
    body: Body,
    target: RequestTarget,
) -> Result<TransportResponse>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let host = url
        .host_str()
        .ok_or_else(|| Error::Config("no host in URL".into()))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| Error::Config(format!("no default port for scheme {}", url.scheme())))?;
    let authority = authority_for(url, host, port);
    let path = path_and_query(url);
    let request_target = match target {
        RequestTarget::OriginForm => path.clone(),
        RequestTarget::AbsoluteForm => url.as_str().to_string(),
    };

    if !contains_header(&headers, "host") {
        headers.insert(0, ("Host".into(), authority));
    }

    // Pick framing strategy: buffered body → Content-Length (including 0);
    // length-known stream → Content-Length; length-unknown stream →
    // Transfer-Encoding: chunked (RFC 9112 §7.1). Callers that already
    // set either header are respected and we trust them.
    let has_cl = contains_header(&headers, "content-length");
    let has_te = contains_header(&headers, "transfer-encoding");

    enum Framing {
        None,
        Buffered(Bytes),
        FixedStream {
            stream: std::pin::Pin<
                Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send + 'static>,
            >,
            length: u64,
        },
        ChunkedStream {
            stream: std::pin::Pin<
                Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send + 'static>,
            >,
        },
    }

    let framing = match body {
        Body::Empty => {
            if method_typically_has_body(method) && !has_cl && !has_te {
                headers.push(("Content-Length".into(), "0".into()));
            }
            Framing::None
        }
        Body::Bytes(b) => {
            if !has_cl && !has_te {
                headers.push(("Content-Length".into(), b.len().to_string()));
            }
            Framing::Buffered(b)
        }
        Body::Stream {
            stream,
            length_hint: Some(length),
        } => {
            if !has_cl && !has_te {
                headers.push(("Content-Length".into(), length.to_string()));
            }
            Framing::FixedStream { stream, length }
        }
        Body::Stream {
            stream,
            length_hint: None,
        } => {
            if !has_te {
                headers.push(("Transfer-Encoding".into(), "chunked".into()));
            }
            Framing::ChunkedStream { stream }
        }
    };

    if !contains_header(&headers, "connection") {
        headers.push(("Connection".into(), "keep-alive".into()));
    }

    // Write request head first.
    let mut req = Vec::new();
    req.extend_from_slice(format!("{method} {request_target} HTTP/1.1\r\n").as_bytes());
    for (name, value) in &headers {
        let name = h1_header_name(name);
        req.extend_from_slice(name.as_bytes());
        req.extend_from_slice(b": ");
        req.extend_from_slice(value.as_bytes());
        req.extend_from_slice(b"\r\n");
    }
    req.extend_from_slice(b"\r\n");
    stream.write_all(&req).await?;

    // Body.
    match framing {
        Framing::None => {}
        Framing::Buffered(b) => {
            stream.write_all(&b).await?;
        }
        Framing::FixedStream {
            stream: mut body_stream,
            length,
        } => {
            let mut sent: u64 = 0;
            while let Some(chunk) = body_stream.next().await {
                let chunk: Bytes = chunk.map_err(Error::Io)?;
                if sent + chunk.len() as u64 > length {
                    return Err(Error::Http(
                        "streaming body exceeded declared content-length".into(),
                    ));
                }
                stream.write_all(&chunk).await?;
                sent += chunk.len() as u64;
            }
            if sent != length {
                return Err(Error::Http(format!(
                    "streaming body ended before declared content-length ({sent}/{length})"
                )));
            }
        }
        Framing::ChunkedStream {
            stream: mut body_stream,
        } => {
            while let Some(chunk) = body_stream.next().await {
                let chunk: Bytes = chunk.map_err(Error::Io)?;
                if chunk.is_empty() {
                    continue;
                }
                let hdr = format!("{:X}\r\n", chunk.len());
                stream.write_all(hdr.as_bytes()).await?;
                stream.write_all(&chunk).await?;
                stream.write_all(b"\r\n").await?;
            }
            stream.write_all(b"0\r\n\r\n").await?;
        }
    }

    stream.flush().await?;

    let (status, resp_headers, body) = read_h1_response(&mut stream, method).await?;
    Ok(TransportResponse {
        status,
        headers: resp_headers,
        body: TransportBody::Buffered(body),
        final_url: url.to_string(),
        version: HttpVersion::Http1_1,
        tls_alpn: None,
        // Populated by the caller for HTTPS (TLS handshake result);
        // always None for plaintext HTTP.
        peer_cert_der: None,
        tls_version: None,
        tls_cipher: None,
    })
}

fn method_typically_has_body(method: &str) -> bool {
    matches!(
        method.to_ascii_uppercase().as_str(),
        "POST" | "PUT" | "PATCH"
    )
}

async fn read_h1_response<S>(
    stream: &mut S,
    method: &str,
) -> Result<(u16, Vec<(String, String)>, Vec<u8>)>
where
    S: AsyncRead + Unpin,
{
    loop {
        let mut buf = read_h1_headers(stream).await?;
        let header_end = find_header_end(&buf)
            .ok_or_else(|| Error::Http("HTTP/1.1 response missing header terminator".into()))?;
        let body_start = header_end + 4;
        let head = String::from_utf8_lossy(&buf[..header_end]);
        let (status, headers) = parse_h1_head(&head)?;
        let initial_body = buf.split_off(body_start);

        if (100..200).contains(&status) && status != 101 {
            continue;
        }

        if method.eq_ignore_ascii_case("HEAD") || matches!(status, 101 | 204 | 304) {
            return Ok((status, headers, Vec::new()));
        }

        let body = if header_contains_token(&headers, "transfer-encoding", "chunked") {
            read_chunked_body(stream, initial_body).await?
        } else if let Some(len) =
            header_first(&headers, "content-length").and_then(|v| v.trim().parse::<usize>().ok())
        {
            read_fixed_body(stream, initial_body, len).await?
        } else {
            read_to_close(stream, initial_body).await?
        };

        return Ok((status, headers, body));
    }
}

async fn read_h1_headers<S>(stream: &mut S) -> Result<Vec<u8>>
where
    S: AsyncRead + Unpin,
{
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 2048];
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Err(Error::Http(
                "connection closed before HTTP/1.1 headers".into(),
            ));
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > MAX_H1_HEADER_BYTES {
            return Err(Error::Http(format!(
                "HTTP/1.1 headers exceed {MAX_H1_HEADER_BYTES} bytes"
            )));
        }
        if find_header_end(&buf).is_some() {
            return Ok(buf);
        }
    }
}

fn parse_h1_head(head: &str) -> Result<(u16, Vec<(String, String)>)> {
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| Error::Http("missing HTTP/1.1 status line".into()))?;
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or_default();
    if !version.starts_with("HTTP/1.") {
        return Err(Error::Http(format!(
            "invalid HTTP/1.1 status line: {status_line}"
        )));
    }
    let status = parts
        .next()
        .ok_or_else(|| Error::Http("missing HTTP status code".into()))?
        .parse::<u16>()
        .map_err(|e| Error::Http(format!("invalid HTTP status code: {e}")))?;

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.push((name.trim().to_string(), value.trim_start().to_string()));
    }
    Ok((status, headers))
}

async fn read_fixed_body<S>(stream: &mut S, mut body: Vec<u8>, len: usize) -> Result<Vec<u8>>
where
    S: AsyncRead + Unpin,
{
    if len > MAX_H1_BODY_BYTES {
        return Err(Error::Http(format!(
            "HTTP/1.1 body exceeds {MAX_H1_BODY_BYTES} bytes"
        )));
    }
    while body.len() < len {
        let remaining = len - body.len();
        let mut tmp = vec![0u8; remaining.min(8192)];
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Err(Error::Http(
                "connection closed before HTTP/1.1 body completed".into(),
            ));
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(len);
    Ok(body)
}

async fn read_to_close<S>(stream: &mut S, mut body: Vec<u8>) -> Result<Vec<u8>>
where
    S: AsyncRead + Unpin,
{
    let mut tmp = [0u8; 8192];
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(body);
        }
        body.extend_from_slice(&tmp[..n]);
        if body.len() > MAX_H1_BODY_BYTES {
            return Err(Error::Http(format!(
                "HTTP/1.1 body exceeds {MAX_H1_BODY_BYTES} bytes"
            )));
        }
    }
}

async fn read_chunked_body<S>(stream: &mut S, mut buf: Vec<u8>) -> Result<Vec<u8>>
where
    S: AsyncRead + Unpin,
{
    let mut out = Vec::new();
    loop {
        let line_end = read_until_crlf(stream, &mut buf).await?;
        let size_line = String::from_utf8_lossy(&buf[..line_end]);
        let size_token = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_token, 16)
            .map_err(|e| Error::Http(format!("invalid chunk size: {e}")))?;
        buf.drain(..line_end + 2);

        if size == 0 {
            read_chunk_trailers(stream, &mut buf).await?;
            return Ok(out);
        }

        read_until_available(stream, &mut buf, size + 2).await?;
        out.extend_from_slice(&buf[..size]);
        if out.len() > MAX_H1_BODY_BYTES {
            return Err(Error::Http(format!(
                "HTTP/1.1 body exceeds {MAX_H1_BODY_BYTES} bytes"
            )));
        }
        if &buf[size..size + 2] != b"\r\n" {
            return Err(Error::Http("chunk missing CRLF terminator".into()));
        }
        buf.drain(..size + 2);
    }
}

async fn read_chunk_trailers<S>(stream: &mut S, buf: &mut Vec<u8>) -> Result<()>
where
    S: AsyncRead + Unpin,
{
    loop {
        let line_end = read_until_crlf(stream, buf).await?;
        let empty = line_end == 0;
        buf.drain(..line_end + 2);
        if empty {
            return Ok(());
        }
    }
}

async fn read_until_crlf<S>(stream: &mut S, buf: &mut Vec<u8>) -> Result<usize>
where
    S: AsyncRead + Unpin,
{
    loop {
        if let Some(pos) = buf.windows(2).position(|w| w == b"\r\n") {
            return Ok(pos);
        }
        read_more(stream, buf).await?;
    }
}

async fn read_until_available<S>(stream: &mut S, buf: &mut Vec<u8>, len: usize) -> Result<()>
where
    S: AsyncRead + Unpin,
{
    while buf.len() < len {
        read_more(stream, buf).await?;
    }
    Ok(())
}

async fn read_more<S>(stream: &mut S, buf: &mut Vec<u8>) -> Result<()>
where
    S: AsyncRead + Unpin,
{
    let mut tmp = [0u8; 8192];
    let n = stream.read(&mut tmp).await?;
    if n == 0 {
        return Err(Error::Http("connection closed during chunked body".into()));
    }
    buf.extend_from_slice(&tmp[..n]);
    if buf.len() > MAX_H1_BODY_BYTES {
        return Err(Error::Http(format!(
            "HTTP/1.1 body exceeds {MAX_H1_BODY_BYTES} bytes"
        )));
    }
    Ok(())
}

fn path_and_query(url: &url::Url) -> String {
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    match url.query() {
        Some(query) => format!("{path}?{query}"),
        None => path.to_string(),
    }
}

fn authority_for(url: &url::Url, host: &str, port: u16) -> String {
    let is_default_port =
        (url.scheme() == "https" && port == 443) || (url.scheme() == "http" && port == 80);
    if is_default_port {
        host.to_string()
    } else {
        format!("{host}:{port}")
    }
}

fn contains_header(headers: &[(String, String)], name: &str) -> bool {
    headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
}

fn h1_header_name(name: &str) -> Cow<'_, str> {
    match name {
        _ if name.eq_ignore_ascii_case("host") => Cow::Borrowed("Host"),
        _ if name.eq_ignore_ascii_case("connection") => Cow::Borrowed("Connection"),
        _ if name.eq_ignore_ascii_case("user-agent") => Cow::Borrowed("User-Agent"),
        _ if name.eq_ignore_ascii_case("accept") => Cow::Borrowed("Accept"),
        _ if name.eq_ignore_ascii_case("accept-encoding") => Cow::Borrowed("Accept-Encoding"),
        _ if name.eq_ignore_ascii_case("accept-language") => Cow::Borrowed("Accept-Language"),
        _ if name.eq_ignore_ascii_case("content-length") => Cow::Borrowed("Content-Length"),
        _ if name.eq_ignore_ascii_case("content-type") => Cow::Borrowed("Content-Type"),
        _ if name.eq_ignore_ascii_case("cookie") => Cow::Borrowed("Cookie"),
        _ if name.eq_ignore_ascii_case("authorization") => Cow::Borrowed("Authorization"),
        _ if name.eq_ignore_ascii_case("proxy-authorization") => {
            Cow::Borrowed("Proxy-Authorization")
        }
        _ if name.eq_ignore_ascii_case("origin") => Cow::Borrowed("Origin"),
        _ if name.eq_ignore_ascii_case("referer") => Cow::Borrowed("Referer"),
        _ if name.eq_ignore_ascii_case("upgrade") => Cow::Borrowed("Upgrade"),
        _ => Cow::Borrowed(name),
    }
}

fn header_first<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn header_contains_token(headers: &[(String, String)], name: &str, token: &str) -> bool {
    header_first(headers, name).is_some_and(|v| {
        v.split(',')
            .any(|part| part.trim().eq_ignore_ascii_case(token))
    })
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn is_h2_alpn_mismatch(err: &Error) -> bool {
    matches!(err, Error::Http(msg) if msg.contains("expected h2"))
}

fn percent_decode(s: &str) -> String {
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

fn base64_encode(input: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
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
