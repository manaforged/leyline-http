//! Split from the parent pool module.
use super::*;

use std::time::Instant;

use crate::HttpVersion;
use crate::trace;

mod body;
mod head;

/// A request body that arrives chunk by chunk.
type BodyStream =
    Pin<Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send + 'static>>;

/// How the request body is delimited on the wire.
enum Framing {
    None,
    Buffered(Bytes),
    Fixed { stream: BodyStream, length: u64 },
    Chunked { stream: BodyStream },
}

/// Serialise and send an HTTP/1.1 request head + body on `stream`.
pub(super) async fn send_h1_request(
    stream: &mut dyn H1Io,
    method: &str,
    url: &url::Url,
    mut headers: Vec<(String, String)>,
    body: H1Body,
    target: H1Target,
) -> Result<bool, H1PooledError> {
    head::validate(method, &headers)?;
    let (request_target, authority) = head::target(url, target)?;

    if !contains_header(&headers, "host") {
        headers.insert(0, ("Host".into(), authority));
    }

    let framing = head::frame(method, &mut headers, body);

    if !contains_header(&headers, "connection") {
        headers.push(("Connection".into(), "keep-alive".into()));
    }

    stream
        .write_all(&head::head(method, &request_target, &headers))
        .await?;

    match framing {
        Framing::None => {}
        Framing::Buffered(b) => {
            stream.write_all(&b).await?;
        }
        Framing::Fixed {
            stream: chunks,
            length,
        } => body::fixed(stream, chunks, length).await?,
        Framing::Chunked { stream: chunks } => body::chunked(stream, chunks).await?,
    }

    stream.flush().await?;

    Ok(header_contains_token(&headers, "connection", "close"))
}
/// Decide whether a keep-alive connection may be reinstated after a response.
pub(super) fn compute_reusable(
    client_asked_close: bool,
    resp_headers: &[(String, String)],
    minor: u8,
) -> bool {
    let server_says_close = header_contains_token(resp_headers, "connection", "close");
    let server_says_keepalive = header_contains_token(resp_headers, "connection", "keep-alive");
    if client_asked_close || server_says_close {
        false
    } else if minor >= 1 {
        true
    } else {
        server_says_keepalive
    }
}
/// Run a single buffered HTTP/1.1 request/response exchange on `stream`.
pub(super) async fn exchange_on_stream(
    stream: &mut dyn H1Io,
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: H1Body,
    target: H1Target,
) -> Result<(WireResponse, bool), H1PooledError> {
    let host = url.host_str().unwrap_or("");
    let started = Instant::now();
    let client_asked_close = send_h1_request(stream, method, url, headers, body, target).await?;
    trace::sent(host, HttpVersion::Http1_1, started.elapsed());
    let started = Instant::now();
    let (status, resp_headers, resp_body, minor) = read_h1_response(stream, method).await?;
    trace::head(host, status, HttpVersion::Http1_1, started.elapsed());
    let reusable = compute_reusable(client_asked_close, &resp_headers, minor);
    Ok((
        WireResponse {
            status,
            headers: resp_headers,
            body: resp_body,
        },
        reusable,
    ))
}
/// Send the request and read only the response head, leaving the body on the wire for a streaming pump.
pub(super) async fn exchange_head_on_stream(
    stream: &mut dyn H1Io,
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: H1Body,
    target: H1Target,
) -> Result<(H1Head, bool), H1PooledError> {
    let host = url.host_str().unwrap_or("");
    let started = Instant::now();
    let client_asked_close = send_h1_request(stream, method, url, headers, body, target).await?;
    trace::sent(host, HttpVersion::Http1_1, started.elapsed());
    let started = Instant::now();
    let head = read_h1_head(stream, method).await?;
    trace::head(host, head.status, HttpVersion::Http1_1, started.elapsed());
    let reusable = compute_reusable(client_asked_close, &head.headers, head.minor)
        && !matches!(head.framing, BodyFraming::ToClose);
    Ok((head, reusable))
}
/// Map a pool error to the `io::Error` the streaming consumer receives.
pub(super) fn h1err_to_io(e: H1PooledError) -> io::Error {
    match e {
        H1PooledError::Io(io) => io,
        H1PooledError::ConnectionClosed(m) => io::Error::new(io::ErrorKind::UnexpectedEof, m),
        other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
    }
}
pub(super) fn method_typically_has_body(method: &str) -> bool {
    ["POST", "PUT", "PATCH"]
        .iter()
        .any(|m| method.eq_ignore_ascii_case(m))
}
/// RFC 9112 §6.1 framing validation.
pub(super) fn validate_framing_headers(headers: &[(String, String)]) -> Result<(), H1PooledError> {
    let cl_count = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .count();
    if cl_count > 1 {
        return Err(H1PooledError::Http(
            "response has multiple Content-Length headers (RFC 9112 §6.1)".into(),
        ));
    }
    if let Some(v) = header_first(headers, "content-length") {
        if v.contains(',') {
            return Err(H1PooledError::Http(
                "response Content-Length contains multiple values".into(),
            ));
        }
        let trimmed = v.trim();
        if trimmed.is_empty()
            || !trimmed.bytes().all(|b| b.is_ascii_digit())
            || trimmed.parse::<u64>().is_err()
        {
            return Err(H1PooledError::Http(format!(
                "response Content-Length `{v}` is not a valid decimal integer (RFC 9112 §8.6)"
            )));
        }
    }

    let te = header_first(headers, "transfer-encoding");
    if te.is_some() && cl_count > 0 {
        return Err(H1PooledError::Http(
            "response has both Content-Length and Transfer-Encoding (RFC 9112 §6.1)".into(),
        ));
    }
    if let Some(te) = te {
        let last = te
            .split(',')
            .map(|t| t.trim())
            .rfind(|t| !t.is_empty())
            .unwrap_or("");
        if !last.eq_ignore_ascii_case("chunked") {
            return Err(H1PooledError::Http(format!(
                "response Transfer-Encoding `{te}`: `chunked` must be the final coding"
            )));
        }
    }
    Ok(())
}
