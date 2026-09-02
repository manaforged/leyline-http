//! Split from the parent pool module.
use super::*;

/// Read and parse the response head only, deciding the body framing but leaving the body on the wire.
pub(super) async fn read_h1_head<S>(stream: &mut S, method: &str) -> Result<H1Head, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut buf = Vec::with_capacity(4096);
    loop {
        let header_end = read_h1_headers(stream, &mut buf).await?;
        let body_start = header_end + 4;
        let head = String::from_utf8_lossy(&buf[..header_end]);
        let (status, headers, minor) = parse_h1_head(&head)?;
        buf.drain(..body_start);

        if (100..200).contains(&status) && status != 101 {
            continue;
        }

        validate_framing_headers(&headers)?;

        let framing = if method.eq_ignore_ascii_case("HEAD") || matches!(status, 101 | 204 | 304) {
            BodyFraming::None
        } else if header_contains_token(&headers, "transfer-encoding", "chunked") {
            BodyFraming::Chunked
        } else if let Some(len) =
            header_first(&headers, "content-length").and_then(|v| v.trim().parse::<u64>().ok())
        {
            BodyFraming::Fixed(len)
        } else {
            BodyFraming::ToClose
        };

        return Ok(H1Head {
            status,
            headers,
            minor,
            framing,
            initial_body: buf,
        });
    }
}
/// Returns `(status, headers, body, http_minor_version)`.
pub(super) async fn read_h1_response<S>(
    stream: &mut S,
    method: &str,
) -> Result<ParsedResponse, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut buf = Vec::with_capacity(4096);
    loop {
        let header_end = read_h1_headers(stream, &mut buf).await?;
        let body_start = header_end + 4;
        let head = String::from_utf8_lossy(&buf[..header_end]);
        let (status, headers, minor) = parse_h1_head(&head)?;
        buf.drain(..body_start);

        if (100..200).contains(&status) && status != 101 {
            continue;
        }

        validate_framing_headers(&headers)?;

        if method.eq_ignore_ascii_case("HEAD") || matches!(status, 101 | 204 | 304) {
            return Ok((status, headers, Vec::new(), minor));
        }

        let body = if header_contains_token(&headers, "transfer-encoding", "chunked") {
            read_chunked_body(stream, buf).await?
        } else if let Some(len) =
            header_first(&headers, "content-length").and_then(|v| v.trim().parse::<usize>().ok())
        {
            read_fixed_body(stream, buf, len).await?
        } else {
            read_to_close(stream, buf).await?
        };

        return Ok((status, headers, body, minor));
    }
}
pub(super) async fn read_h1_headers<S>(
    stream: &mut S,
    buf: &mut Vec<u8>,
) -> Result<usize, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut tmp = [0u8; 2048];
    loop {
        if let Some(header_end) = find_header_end(buf) {
            if header_end + 4 > MAX_H1_HEADER_BYTES {
                return Err(H1PooledError::Http(format!(
                    "HTTP/1.1 headers exceed {MAX_H1_HEADER_BYTES} bytes"
                )));
            }
            return Ok(header_end);
        }
        if buf.len() > MAX_H1_HEADER_BYTES {
            return Err(H1PooledError::Http(format!(
                "HTTP/1.1 headers exceed {MAX_H1_HEADER_BYTES} bytes"
            )));
        }
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Err(H1PooledError::ConnectionClosed(
                "before HTTP/1.1 headers".into(),
            ));
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}
/// Parse an HTTP/1.1 response head into `(status, headers, minor version)`.
pub fn parse_h1_head(head: &str) -> Result<ParsedHead, H1PooledError> {
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| H1PooledError::Http("missing HTTP/1.1 status line".into()))?;
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or_default();
    if !version.starts_with("HTTP/1.") {
        return Err(H1PooledError::Http(format!(
            "invalid HTTP/1.1 status line: {status_line}"
        )));
    }
    let minor = match version.as_bytes().get(7) {
        Some(b'0') => 0,
        Some(b'1') => 1,
        _ => {
            return Err(H1PooledError::Http(format!(
                "unknown HTTP/1.x minor version in status line: {status_line}"
            )));
        }
    };
    let status = parts
        .next()
        .ok_or_else(|| H1PooledError::Http("missing HTTP status code".into()))?
        .parse::<u16>()
        .map_err(|e| H1PooledError::Http(format!("invalid HTTP status code: {e}")))?;

    let mut headers: Vec<(String, String)> = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some((_, value)) = headers.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.is_empty() || name.ends_with(' ') || name.ends_with('\t') {
            continue;
        }
        headers.push((name.to_string(), value.trim_start().to_string()));
    }
    Ok((status, headers, minor))
}
pub(super) async fn read_fixed_body<S>(
    stream: &mut S,
    mut body: Vec<u8>,
    len: usize,
) -> Result<Vec<u8>, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    if len > MAX_H1_BODY_BYTES {
        return Err(H1PooledError::Http(format!(
            "HTTP/1.1 body exceeds {MAX_H1_BODY_BYTES} bytes"
        )));
    }
    while body.len() < len {
        let remaining = len - body.len();
        let mut tmp = vec![0u8; remaining.min(8192)];
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Err(H1PooledError::ConnectionClosed(
                "before HTTP/1.1 body completed".into(),
            ));
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(len);
    Ok(body)
}
pub(super) async fn read_to_close<S>(
    stream: &mut S,
    mut body: Vec<u8>,
) -> Result<Vec<u8>, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut tmp = [0u8; 8192];
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(body);
        }
        body.extend_from_slice(&tmp[..n]);
        if body.len() > MAX_H1_BODY_BYTES {
            return Err(H1PooledError::Http(format!(
                "HTTP/1.1 body exceeds {MAX_H1_BODY_BYTES} bytes"
            )));
        }
    }
}
/// Decode a `Transfer-Encoding: chunked` body from `stream`, starting with the bytes already in `buf`.
pub async fn read_chunked_body<S>(
    stream: &mut S,
    mut buf: Vec<u8>,
) -> Result<Vec<u8>, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut out = Vec::new();
    loop {
        let line_end = read_until_crlf(stream, &mut buf).await?;
        let size_line = String::from_utf8_lossy(&buf[..line_end]);
        let size_token = size_line.split(';').next().unwrap_or("").trim();
        let size_u64 = u64::from_str_radix(size_token, 16)
            .map_err(|e| H1PooledError::Http(format!("invalid chunk size: {e}")))?;
        if size_u64 > MAX_H1_BODY_BYTES as u64 {
            return Err(H1PooledError::Http(format!(
                "HTTP/1.1 chunk size {size_u64} exceeds {MAX_H1_BODY_BYTES}-byte body cap"
            )));
        }
        let size = size_u64 as usize;
        buf.drain(..line_end + 2);

        if size == 0 {
            read_chunk_trailers(stream, &mut buf).await?;
            return Ok(out);
        }

        read_until_available(stream, &mut buf, size + 2).await?;
        out.extend_from_slice(&buf[..size]);
        if out.len() > MAX_H1_BODY_BYTES {
            return Err(H1PooledError::Http(format!(
                "HTTP/1.1 body exceeds {MAX_H1_BODY_BYTES} bytes"
            )));
        }
        if &buf[size..size + 2] != b"\r\n" {
            return Err(H1PooledError::Http("chunk missing CRLF terminator".into()));
        }
        buf.drain(..size + 2);
    }
}
