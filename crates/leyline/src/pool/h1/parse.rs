use super::*;
use crate::core::session::decompress::BodyLimit;

pub(super) async fn read_h1_head<S>(
    stream: &mut S,
    method: &str,
    mut buf: Vec<u8>,
) -> Result<H1Head, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut informational = 0usize;
    let mut head_bytes = 0usize;
    loop {
        let header_end = read_h1_headers(stream, &mut buf).await?;
        let body_start = header_end + 4;
        head_bytes += body_start;
        if head_bytes > MAX_H1_HEADER_BYTES {
            return Err(H1PooledError::Http(format!(
                "HTTP/1.1 headers exceed {MAX_H1_HEADER_BYTES} bytes"
            )));
        }
        let head = String::from_utf8_lossy(&buf[..header_end]);
        let (status, headers, minor) = parse_h1_head(&head)?;
        buf.drain(..body_start);

        if status == 101 {
            return Err(H1PooledError::Http(
                "unexpected 101 Switching Protocols".into(),
            ));
        }
        if (100..200).contains(&status) {
            informational += 1;
            if informational > MAX_H1_INFORMATIONAL {
                return Err(H1PooledError::Http(format!(
                    "more than {MAX_H1_INFORMATIONAL} informational responses"
                )));
            }
            continue;
        }

        validate_framing_headers(&headers)?;

        let framing = if method.eq_ignore_ascii_case("HEAD") || matches!(status, 204 | 304) {
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

pub(super) async fn read_h1_body<S>(
    stream: &mut S,
    head: &mut H1Head,
    limit: usize,
) -> Result<(Vec<u8>, bool), H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let buf = std::mem::take(&mut head.initial_body);
    let excess = has_excess(head.framing, buf.len());
    let body = match head.framing {
        BodyFraming::None => Vec::new(),
        BodyFraming::Chunked => read_chunked_body(stream, buf, limit).await?,
        BodyFraming::Fixed(len) => {
            let len = usize::try_from(len).map_err(|e| oversized(&e, limit))?;
            read_fixed_body(stream, buf, len, limit).await?
        }
        BodyFraming::ToClose => read_to_close(stream, buf, limit).await?,
    };
    Ok((body, excess))
}

pub(super) fn has_excess(framing: BodyFraming, buffered: usize) -> bool {
    match framing {
        BodyFraming::None => buffered > 0,
        BodyFraming::Fixed(len) => buffered as u64 > len,
        BodyFraming::Chunked | BodyFraming::ToClose => false,
    }
}

fn bare_lf(head: &[u8]) -> bool {
    head.iter()
        .enumerate()
        .any(|(i, &b)| b == b'\n' && (i == 0 || head[i - 1] != b'\r'))
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
        let scanned = find_header_end(buf).map_or(buf.len(), |end| end + 4);
        if bare_lf(&buf[..scanned]) {
            return Err(H1PooledError::Http(
                "HTTP/1.1 head has a bare LF line ending".into(),
            ));
        }
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
            return Err(H1PooledError::Http(format!(
                "HTTP/1.1 header line has no colon: {line}"
            )));
        };
        if name.is_empty() || name.ends_with(' ') || name.ends_with('\t') {
            return Err(H1PooledError::Http(format!(
                "invalid HTTP/1.1 header name: {name}"
            )));
        }
        headers.push((name.to_string(), value.trim_start().to_string()));
    }
    Ok((status, headers, minor))
}
pub(super) async fn read_fixed_body<S>(
    stream: &mut S,
    mut body: Vec<u8>,
    len: usize,
    limit: usize,
) -> Result<Vec<u8>, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    if len > limit {
        return Err(body_too_large(limit));
    }
    read_fixed_into(stream, &mut body, len).await?;
    body.truncate(len);
    Ok(body)
}

pub(super) async fn read_fixed_into<S>(
    stream: &mut S,
    body: &mut Vec<u8>,
    len: usize,
) -> Result<(), H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
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
    Ok(())
}
pub(super) async fn read_to_close<S>(
    stream: &mut S,
    mut body: Vec<u8>,
    limit: usize,
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
        if body.len() > limit {
            return Err(body_too_large(limit));
        }
    }
}
pub async fn read_chunked_body<S>(
    stream: &mut S,
    mut buf: Vec<u8>,
    limit: usize,
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
        let size = usize::try_from(size_u64).map_err(|e| oversized(&e, limit))?;
        let total = out
            .len()
            .checked_add(size)
            .ok_or_else(|| body_too_large(limit))?;
        if total > limit {
            return Err(body_too_large(limit));
        }
        buf.drain(..line_end + 2);

        if size == 0 {
            read_chunk_trailers(stream, &mut buf).await?;
            return Ok(out);
        }

        let chunk_end = size.checked_add(2).ok_or_else(|| body_too_large(limit))?;
        read_until_available(stream, &mut buf, chunk_end).await?;
        out.extend_from_slice(&buf[..size]);
        if &buf[size..chunk_end] != b"\r\n" {
            return Err(H1PooledError::Http("chunk missing CRLF terminator".into()));
        }
        buf.drain(..chunk_end);
    }
}

fn oversized(e: &std::num::TryFromIntError, limit: usize) -> H1PooledError {
    tracing::debug!(error = %e, "h1 body length does not fit in usize");
    body_too_large(limit)
}

fn body_too_large(limit: usize) -> H1PooledError {
    H1PooledError::Io(BodyLimit::session(limit).into_io())
}
