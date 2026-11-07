use super::*;
use crate::error::BodyLimit;
use crate::wire::{Fields, ResponseState, Seen, has_body};

pub(super) async fn read_h1_head<S>(
    stream: &mut S,
    method: &Method,
    mut buf: Vec<u8>,
) -> Result<H1Head, H1Error>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut state = ResponseState::default();
    let mut head_bytes = 0usize;
    loop {
        let header_end = read_h1_headers(stream, &mut buf).await?;
        let body_start = header_end + 4;
        head_bytes += body_start;
        if head_bytes > MAX_H1_HEADER_BYTES {
            return Err(malformed(format!(
                "HTTP/1.1 headers exceed {MAX_H1_HEADER_BYTES} bytes"
            ))
            .into());
        }
        let head = &buf[..header_end];
        let Some((status, minor, framing, seen)) = final_head(head, method, &mut state)? else {
            buf.drain(..body_start);
            continue;
        };
        let headers = Fields::raw(Bytes::copy_from_slice(head), seen);
        buf.drain(..body_start);
        return Ok(H1Head {
            status,
            headers,
            minor,
            framing,
            initial_body: buf,
        });
    }
}

fn final_head(
    head: &[u8],
    method: &Method,
    state: &mut ResponseState,
) -> Result<Option<(u16, u8, BodyFraming, Seen)>, Error> {
    let (status, minor, scanned) = classify(head, state)?;
    if *state != ResponseState::Final {
        return Ok(None);
    }
    scanned.body.validate()?;
    let framing = if has_body(*method == Method::HEAD, status) {
        scanned.body.framing()
    } else {
        BodyFraming::None
    };
    Ok(Some((status, minor, framing, scanned.seen())))
}

pub(super) fn classify<'h>(
    head: &'h [u8],
    state: &mut ResponseState,
) -> Result<(u16, u8, Scanned<'h>), Error> {
    let (status, minor, scanned) = scan_head(head)?;
    let code = http::StatusCode::from_u16(status).map_err(|e| malformed(e.to_string()))?;
    state.head(code).map_err(malformed)?;
    Ok((status, minor, scanned))
}

pub(super) struct Buffered {
    pub(super) body: Vec<u8>,
    pub(super) trailers: http::HeaderMap,
    pub(super) excess: bool,
}

pub(super) async fn read_h1_body<S>(
    stream: &mut S,
    head: &mut H1Head,
    limit: BodyLimit,
) -> Result<Buffered, H1Error>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let excess = has_excess(head.framing, head.initial_body.len());
    let mut trailers = http::HeaderMap::new();
    if let Some(body) = buffered_body(head, limit) {
        return Ok(Buffered {
            body,
            trailers,
            excess,
        });
    }
    let buf = std::mem::take(&mut head.initial_body);
    let body = match head.framing {
        BodyFraming::None => Vec::new(),
        BodyFraming::Chunked => {
            let (body, sent) = read_chunked(stream, buf, limit).await?;
            trailers = sent;
            body
        }
        BodyFraming::Fixed(len) => {
            let len = usize::try_from(len).map_err(|e| oversized(&e, limit))?;
            read_fixed_body(stream, buf, len, limit).await?
        }
        BodyFraming::ToClose => read_to_close(stream, buf, limit).await?,
    };
    Ok(Buffered {
        body,
        trailers,
        excess,
    })
}

fn buffered_body(head: &mut H1Head, limit: BodyLimit) -> Option<Vec<u8>> {
    let len = match head.framing {
        BodyFraming::None => 0,
        BodyFraming::Fixed(len) => usize::try_from(len)
            .ok()
            .filter(|len| limit.admit(0, *len).is_ok())?,
        BodyFraming::Chunked | BodyFraming::ToClose => return None,
    };
    let body = head.initial_body.get(..len)?.to_vec();
    head.initial_body.clear();
    Some(body)
}

pub(super) fn has_excess(framing: BodyFraming, buffered: usize) -> bool {
    match framing {
        BodyFraming::None => buffered > 0,
        BodyFraming::Fixed(len) => buffered as u64 > len,
        BodyFraming::Chunked | BodyFraming::ToClose => false,
    }
}

fn bare_lf(head: &[u8]) -> bool {
    memchr::memchr_iter(b'\n', head).any(|i| i == 0 || head[i - 1] != b'\r')
}
pub(super) async fn read_h1_headers<S>(stream: &mut S, buf: &mut Vec<u8>) -> Result<usize, H1Error>
where
    S: AsyncRead + Unpin + ?Sized,
{
    loop {
        let end = find_header_end(buf);
        let scanned = end.map_or(buf.len(), |end| end + 4);
        if bare_lf(&buf[..scanned]) {
            return Err(malformed("HTTP/1.1 head has a bare LF line ending").into());
        }
        if let Some(header_end) = end {
            if header_end + 4 > MAX_H1_HEADER_BYTES {
                return Err(malformed(format!(
                    "HTTP/1.1 headers exceed {MAX_H1_HEADER_BYTES} bytes"
                ))
                .into());
            }
            return Ok(header_end);
        }
        if buf.len() > MAX_H1_HEADER_BYTES {
            return Err(malformed(format!(
                "HTTP/1.1 headers exceed {MAX_H1_HEADER_BYTES} bytes"
            ))
            .into());
        }
        if read::read_into(stream, buf, 2048).await? == 0 {
            return Err(closed("before HTTP/1.1 headers"));
        }
    }
}
pub(super) async fn read_fixed_body<S>(
    stream: &mut S,
    mut body: Vec<u8>,
    len: usize,
    limit: BodyLimit,
) -> Result<Vec<u8>, H1Error>
where
    S: AsyncRead + Unpin + ?Sized,
{
    limit.admit(0, len).map_err(BodyLimit::error)?;
    read_fixed_into(stream, &mut body, len).await?;
    body.truncate(len);
    Ok(body)
}

pub(super) async fn read_fixed_into<S>(
    stream: &mut S,
    body: &mut Vec<u8>,
    len: usize,
) -> Result<(), H1Error>
where
    S: AsyncRead + Unpin + ?Sized,
{
    while body.len() < len {
        let remaining = len - body.len();
        if read::read_into(stream, body, remaining.min(READ_CHUNK)).await? == 0 {
            return Err(cut_short());
        }
    }
    Ok(())
}
pub(super) async fn read_to_close<S>(
    stream: &mut S,
    mut body: Vec<u8>,
    limit: BodyLimit,
) -> Result<Vec<u8>, H1Error>
where
    S: AsyncRead + Unpin + ?Sized,
{
    loop {
        if read::read_into(stream, &mut body, READ_CHUNK).await? == 0 {
            return Ok(body);
        }
        limit.admit(body.len(), 0).map_err(BodyLimit::error)?;
    }
}

#[cfg(feature = "bench-internals")]
/// Reads a chunked body from `stream` after the `buf` bytes already read, up to `limit` bytes.
pub async fn read_chunked_body<S>(
    stream: &mut S,
    buf: Vec<u8>,
    limit: usize,
) -> Result<Vec<u8>, Error>
where
    S: AsyncRead + Unpin + ?Sized,
{
    read_chunked(stream, buf, BodyLimit::session(limit))
        .await
        .map(|(body, _)| body)
        .map_err(H1Error::into_error)
}

async fn read_chunked<S>(
    stream: &mut S,
    mut buf: Vec<u8>,
    limit: BodyLimit,
) -> Result<(Vec<u8>, http::HeaderMap), H1Error>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut out = Vec::new();
    let mut chunked = Chunked::new();
    loop {
        match chunked.next(stream, &mut buf).await? {
            Step::Size(size) => {
                let size = usize::try_from(size).map_err(|e| oversized(&e, limit))?;
                limit.admit(out.len(), size).map_err(BodyLimit::error)?;
            }
            Step::Data(n) => {
                out.extend_from_slice(&buf[..n]);
                buf.drain(..n);
            }
            Step::Done => return Ok((out, chunked.into_trailers())),
        }
    }
}

fn oversized(e: &std::num::TryFromIntError, limit: BodyLimit) -> Error {
    tracing::debug!(error = %e, "h1 body length does not fit in usize");
    limit.error()
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
