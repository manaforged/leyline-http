use super::*;
use crate::core::session::decompress::BodyLimit;
use crate::util::MAX_INFORMATIONAL;

pub(super) async fn read_h1_head<S>(
    stream: &mut S,
    method: &str,
    mut buf: Vec<u8>,
) -> Result<H1Head, Error>
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
            return Err(malformed(format!(
                "HTTP/1.1 headers exceed {MAX_H1_HEADER_BYTES} bytes"
            )));
        }
        let head = Bytes::copy_from_slice(&buf[..header_end]);
        let (status, headers, minor, body) = parse_head(&head)?;
        buf.drain(..body_start);

        if status == 101 {
            return Err(malformed("unexpected 101 Switching Protocols"));
        }
        if (100..200).contains(&status) {
            informational += 1;
            if informational > MAX_INFORMATIONAL {
                return Err(malformed(format!(
                    "more than {MAX_INFORMATIONAL} informational responses"
                )));
            }
            continue;
        }

        body.validate()?;

        let framing = if method.eq_ignore_ascii_case("HEAD") || matches!(status, 204 | 304) {
            BodyFraming::None
        } else {
            body.framing()
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
) -> Result<(Vec<u8>, bool), Error>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let excess = has_excess(head.framing, head.initial_body.len());
    if let Some(body) = buffered_body(head, limit) {
        return Ok((body, excess));
    }
    let buf = std::mem::take(&mut head.initial_body);
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

fn buffered_body(head: &mut H1Head, limit: usize) -> Option<Vec<u8>> {
    let len = match head.framing {
        BodyFraming::None => 0,
        BodyFraming::Fixed(len) => usize::try_from(len).ok().filter(|len| *len <= limit)?,
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
pub(super) async fn read_h1_headers<S>(stream: &mut S, buf: &mut Vec<u8>) -> Result<usize, Error>
where
    S: AsyncRead + Unpin + ?Sized,
{
    loop {
        let end = find_header_end(buf);
        let scanned = end.map_or(buf.len(), |end| end + 4);
        if bare_lf(&buf[..scanned]) {
            return Err(malformed("HTTP/1.1 head has a bare LF line ending"));
        }
        if let Some(header_end) = end {
            if header_end + 4 > MAX_H1_HEADER_BYTES {
                return Err(malformed(format!(
                    "HTTP/1.1 headers exceed {MAX_H1_HEADER_BYTES} bytes"
                )));
            }
            return Ok(header_end);
        }
        if buf.len() > MAX_H1_HEADER_BYTES {
            return Err(malformed(format!(
                "HTTP/1.1 headers exceed {MAX_H1_HEADER_BYTES} bytes"
            )));
        }
        if read::read_into(stream, buf, 2048).await? == 0 {
            return Err(closed("before HTTP/1.1 headers"));
        }
    }
}
pub fn parse_h1_head(head: &[u8]) -> Result<(u16, HeaderMap, u8), Error> {
    let (status, headers, minor, _) = parse_head(&Bytes::copy_from_slice(head))?;
    Ok((status, headers, minor))
}

pub(super) fn parse_head(head: &Bytes) -> Result<(u16, HeaderMap, u8, BodyHeaders), Error> {
    let mut sink = Typed {
        head,
        fields: HeaderMap::with_capacity(memchr::memchr_iter(b'\n', head).count()),
        last: None,
        body: BodyHeaders::default(),
    };
    let (status, minor) = parse(head, &mut sink)?;
    sink.flush();
    Ok((status, sink.fields, minor, sink.body))
}

#[derive(Default)]
pub(super) struct BodyHeaders {
    lengths: usize,
    length: Option<http::HeaderValue>,
    coding: Option<http::HeaderValue>,
}

impl BodyHeaders {
    fn note(&mut self, name: &http::HeaderName, value: &http::HeaderValue) {
        if name == http::header::CONTENT_LENGTH {
            self.lengths += 1;
            self.length.get_or_insert_with(|| value.clone());
        } else if name == http::header::TRANSFER_ENCODING {
            self.coding.get_or_insert_with(|| value.clone());
        }
    }

    fn length(&self) -> Option<&str> {
        self.length.as_ref().and_then(|v| v.to_str().ok())
    }

    fn coding(&self) -> Option<&str> {
        self.coding.as_ref().and_then(|v| v.to_str().ok())
    }

    fn validate(&self) -> Result<(), Error> {
        if self.lengths > 1 {
            return Err(malformed(
                "response has multiple Content-Length headers (RFC 9112 §6.1)",
            ));
        }
        if let Some(v) = self.length() {
            if v.contains(',') {
                return Err(malformed(
                    "response Content-Length contains multiple values",
                ));
            }
            if crate::util::content_length(v).is_none() {
                return Err(malformed(format!(
                    "response Content-Length `{v}` is not a valid decimal integer (RFC 9112 §8.6)"
                )));
            }
        }
        let te = self.coding();
        if te.is_some() && self.lengths > 0 {
            return Err(malformed(
                "response has both Content-Length and Transfer-Encoding (RFC 9112 §6.1)",
            ));
        }
        if let Some(te) = te {
            let last = te
                .split(',')
                .map(|t| t.trim())
                .rfind(|t| !t.is_empty())
                .unwrap_or("");
            if !last.eq_ignore_ascii_case("chunked") {
                return Err(malformed(format!(
                    "response Transfer-Encoding `{te}`: `chunked` must be the final coding"
                )));
            }
        }
        Ok(())
    }

    fn framing(&self) -> BodyFraming {
        if self.coding().is_some_and(|te| has_token(te, "chunked")) {
            BodyFraming::Chunked
        } else if let Some(len) = self.length().and_then(crate::util::content_length) {
            BodyFraming::Fixed(len)
        } else {
            BodyFraming::ToClose
        }
    }
}

trait Sink {
    fn field(&mut self, name: &[u8], value: &[u8]) -> Result<(), Error>;
    fn fold(&mut self, line: &[u8]) -> Result<(), Error>;
}

struct Typed<'a> {
    head: &'a Bytes,
    fields: HeaderMap,
    last: Option<(http::HeaderName, http::HeaderValue)>,
    body: BodyHeaders,
}

impl Typed<'_> {
    fn flush(&mut self) {
        if let Some((name, value)) = self.last.take() {
            self.body.note(&name, &value);
            self.fields.append(name, value);
        }
    }
}

impl Sink for Typed<'_> {
    fn field(&mut self, name: &[u8], value: &[u8]) -> Result<(), Error> {
        self.flush();
        let value = self.head.slice_ref(trim_start(value));
        let Ok(name) = http::HeaderName::from_bytes(name) else {
            return Ok(());
        };
        match http::HeaderValue::from_maybe_shared(value) {
            Ok(value) => {
                self.last = Some((name, value));
                Ok(())
            }
            Err(_) => framing_value(&name),
        }
    }

    fn fold(&mut self, line: &[u8]) -> Result<(), Error> {
        let Some((name, value)) = self.last.as_mut() else {
            return Ok(());
        };
        let mut joined = value.as_bytes().to_vec();
        joined.push(b' ');
        joined.extend_from_slice(trim_end(trim_start(line)));
        if let Ok(folded) = http::HeaderValue::from_maybe_shared(Bytes::from(joined)) {
            *value = folded;
            return Ok(());
        }
        framing_value(name)?;
        self.last = None;
        Ok(())
    }
}

fn framing_value(name: &http::HeaderName) -> Result<(), Error> {
    if name == http::header::CONTENT_LENGTH || name == http::header::TRANSFER_ENCODING {
        return Err(malformed(format!(
            "response {name} value has a control character"
        )));
    }
    Ok(())
}

fn trim_start(mut bytes: &[u8]) -> &[u8] {
    while let Some(n) = edge_space(bytes, true) {
        bytes = &bytes[n..];
    }
    bytes
}

fn trim_end(mut bytes: &[u8]) -> &[u8] {
    while let Some(n) = edge_space(bytes, false) {
        bytes = &bytes[..bytes.len() - n];
    }
    bytes
}

fn edge_space(bytes: &[u8], front: bool) -> Option<usize> {
    let edge = if front { bytes.first() } else { bytes.last() };
    if let Some(&byte) = edge.filter(|byte| byte.is_ascii()) {
        return char::from(byte).is_whitespace().then_some(1);
    }
    let part = |n: usize| {
        let range = if front {
            0..n
        } else {
            bytes.len() - n..bytes.len()
        };
        std::str::from_utf8(&bytes[range]).ok()
    };
    let n = (1..=bytes.len().min(4)).find(|&n| part(n).is_some())?;
    let text = part(n)?;
    let ch = if front {
        text.chars().next()
    } else {
        text.chars().next_back()
    };
    ch.filter(|c| c.is_whitespace()).map(|_| n)
}

fn lines(head: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = Some(head);
    std::iter::from_fn(move || {
        let current = rest?;
        match memchr::memchr_iter(b'\n', current)
            .find(|&i| i > 0 && current[i - 1] == b'\r')
            .map(|i| i - 1)
        {
            Some(end) => {
                rest = Some(&current[end + 2..]);
                Some(&current[..end])
            }
            None => {
                rest = None;
                Some(current)
            }
        }
    })
}

fn status_line(line: &[u8]) -> Result<(u16, u8), Error> {
    let shown = || String::from_utf8_lossy(line);
    let mut parts = line.splitn(3, |&b| b == b' ');
    let version = parts.next().unwrap_or_default();
    let minor = match version {
        b"HTTP/1.0" => 0,
        b"HTTP/1.1" => 1,
        _ => {
            return Err(malformed(format!(
                "invalid HTTP/1.x status line: {}",
                shown()
            )));
        }
    };
    let code = parts
        .next()
        .ok_or_else(|| malformed("missing HTTP status code"))?;
    let status = match code {
        [a @ b'1'..=b'9', b @ b'0'..=b'9', c @ b'0'..=b'9'] => {
            u16::from(a - b'0') * 100 + u16::from(b - b'0') * 10 + u16::from(c - b'0')
        }
        _ => {
            return Err(malformed(
                "HTTP status code must contain exactly three digits",
            ));
        }
    };
    Ok((status, minor))
}

fn parse<S: Sink>(head: &[u8], sink: &mut S) -> Result<(u16, u8), Error> {
    let mut lines = lines(head);
    let first = lines
        .next()
        .ok_or_else(|| malformed("missing HTTP/1.1 status line"))?;
    let (status, minor) = status_line(first)?;
    for line in lines {
        match line.first() {
            None => continue,
            Some(b' ' | b'\t') => {
                sink.fold(line)?;
                continue;
            }
            Some(_) => {}
        }
        let mut halves = line.splitn(2, |&b| b == b':');
        let (Some(name), Some(value)) = (halves.next(), halves.next()) else {
            return Err(malformed(format!(
                "HTTP/1.1 header line has no colon: {}",
                String::from_utf8_lossy(line)
            )));
        };
        if name.is_empty() || name.ends_with(b" ") || name.ends_with(b"\t") {
            return Err(malformed(format!(
                "invalid HTTP/1.1 header name: {}",
                String::from_utf8_lossy(name)
            )));
        }
        sink.field(name, value)?;
    }
    Ok((status, minor))
}
pub(super) async fn read_fixed_body<S>(
    stream: &mut S,
    mut body: Vec<u8>,
    len: usize,
    limit: usize,
) -> Result<Vec<u8>, Error>
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
) -> Result<(), Error>
where
    S: AsyncRead + Unpin + ?Sized,
{
    while body.len() < len {
        let remaining = len - body.len();
        if read::read_into(stream, body, remaining.min(8192)).await? == 0 {
            return Err(closed("before HTTP/1.1 body completed"));
        }
    }
    Ok(())
}
pub(super) async fn read_to_close<S>(
    stream: &mut S,
    mut body: Vec<u8>,
    limit: usize,
) -> Result<Vec<u8>, Error>
where
    S: AsyncRead + Unpin + ?Sized,
{
    loop {
        if read::read_into(stream, &mut body, 8192).await? == 0 {
            return Ok(body);
        }
        if body.len() > limit {
            return Err(body_too_large(limit));
        }
    }
}
pub async fn read_chunked_body<S>(
    stream: &mut S,
    mut buf: Vec<u8>,
    limit: usize,
) -> Result<Vec<u8>, Error>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut out = Vec::new();
    loop {
        let line_end = read_until_crlf(stream, &mut buf).await?;
        let size_u64 = chunk_size(&buf[..line_end]).map_err(malformed)?;
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
            return Err(malformed("chunk missing CRLF terminator"));
        }
        buf.drain(..chunk_end);
    }
}

fn oversized(e: &std::num::TryFromIntError, limit: usize) -> Error {
    tracing::debug!(error = %e, "h1 body length does not fit in usize");
    body_too_large(limit)
}

fn body_too_large(limit: usize) -> Error {
    BodyLimit::session(limit).error()
}

pub(super) fn chunk_size(line: &[u8]) -> Result<u64, String> {
    let line = String::from_utf8_lossy(line);
    let token = line.split(';').next().unwrap_or("").trim();
    if token.is_empty() || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("invalid chunk size: {token:?}"));
    }
    u64::from_str_radix(token, 16).map_err(|e| format!("invalid chunk size: {e}"))
}
