//! Split from the parent pool module.
use super::*;

/// Serialise and send an HTTP/1.1 request head + body on `stream`.
pub(super) async fn send_h1_request(
    stream: &mut dyn H1Io,
    method: &str,
    url: &url::Url,
    mut headers: Vec<(String, String)>,
    body: H1Body,
    target: H1Target,
) -> Result<bool, H1PooledError> {
    if !is_valid_token(method) {
        return Err(H1PooledError::Config(format!(
            "invalid HTTP method `{method}`: non-token bytes not allowed"
        )));
    }
    for (name, value) in &headers {
        if !is_valid_token(name) {
            return Err(H1PooledError::Config(format!(
                "invalid header name `{name}`: non-token bytes not allowed"
            )));
        }
        if !is_valid_header_value(value) {
            return Err(H1PooledError::Config(format!(
                "invalid value for header `{name}`: control characters not allowed"
            )));
        }
    }
    let host = url
        .host_str()
        .ok_or_else(|| H1PooledError::Config("no host in URL".into()))?;
    let port = url.port_or_known_default().ok_or_else(|| {
        H1PooledError::Config(format!("no default port for scheme {}", url.scheme()))
    })?;
    let authority = authority_for(url, host, port);
    let path = path_and_query(url);
    let request_target = match target {
        H1Target::OriginForm => path,
        H1Target::AbsoluteForm => {
            format!("{}://{}{}", url.scheme(), authority, path)
        }
    };
    if !is_valid_request_target(&request_target) {
        return Err(H1PooledError::Config(format!(
            "invalid request target `{request_target}`: control characters not allowed"
        )));
    }

    if !contains_header(&headers, "host") {
        headers.insert(0, ("Host".into(), authority));
    }

    let has_cl = contains_header(&headers, "content-length");
    let has_te = contains_header(&headers, "transfer-encoding");

    enum Framing {
        None,
        Buffered(Bytes),
        FixedStream {
            stream:
                Pin<Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send + 'static>>,
            length: u64,
        },
        ChunkedStream {
            stream:
                Pin<Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send + 'static>>,
        },
    }

    let framing = match body {
        H1Body::Empty => {
            if method_typically_has_body(method) && !has_cl && !has_te {
                headers.push(("Content-Length".into(), "0".into()));
            }
            Framing::None
        }
        H1Body::Buffered(b) => {
            if !has_cl && !has_te {
                headers.push(("Content-Length".into(), b.len().to_string()));
            }
            Framing::Buffered(b)
        }
        H1Body::FixedStream { stream, length } => {
            if !has_cl && !has_te {
                headers.push(("Content-Length".into(), length.to_string()));
            }
            Framing::FixedStream { stream, length }
        }
        H1Body::ChunkedStream { stream } => {
            if !has_te {
                headers.push(("Transfer-Encoding".into(), "chunked".into()));
            }
            Framing::ChunkedStream { stream }
        }
    };

    if !contains_header(&headers, "connection") {
        headers.push(("Connection".into(), "keep-alive".into()));
    }

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
                let chunk: Bytes = chunk?;
                if sent + chunk.len() as u64 > length {
                    return Err(H1PooledError::Http(
                        "streaming body exceeded declared content-length".into(),
                    ));
                }
                stream.write_all(&chunk).await?;
                sent += chunk.len() as u64;
            }
            if sent != length {
                return Err(H1PooledError::Http(format!(
                    "streaming body ended before declared content-length ({sent}/{length})"
                )));
            }
        }
        Framing::ChunkedStream {
            stream: mut body_stream,
        } => {
            while let Some(chunk) = body_stream.next().await {
                let chunk: Bytes = chunk?;
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
    let client_asked_close = send_h1_request(stream, method, url, headers, body, target).await?;
    let (status, resp_headers, resp_body, minor) = read_h1_response(stream, method).await?;
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
    let client_asked_close = send_h1_request(stream, method, url, headers, body, target).await?;
    let head = read_h1_head(stream, method).await?;
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
