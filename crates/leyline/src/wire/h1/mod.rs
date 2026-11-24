use std::io;

use bytes::Bytes;
use http::HeaderMap;

use super::{HeaderPair, Seen};
use crate::error::{Error, Kind};
use crate::util::has_token;

mod scan;
pub(crate) use scan::{Scanned, find_values, scan_head};

/// The largest HTTP/1.1 response head leyline reads.
pub const MAX_H1_HEADER_BYTES: usize = 64 * 1024;

#[derive(Debug)]
struct Malformed(String);

impl std::fmt::Display for Malformed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "http: {}", self.0)
    }
}

impl std::error::Error for Malformed {}

pub(crate) fn malformed(message: impl Into<String>) -> Error {
    let message = message.into();
    let source = io::Error::new(io::ErrorKind::InvalidData, Malformed(message.clone()));
    Error::new(Kind::Io)
        .with_message(message)
        .with_source(source)
}

pub(crate) fn is_malformed(error: &Error) -> bool {
    error
        .io()
        .and_then(io::Error::get_ref)
        .is_some_and(|inner| inner.is::<Malformed>())
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum BodyFraming {
    None,
    Fixed(u64),
    Chunked,
    ToClose,
}

#[cfg(any(test, feature = "websocket", feature = "bench-internals"))]
pub(crate) fn parse_h1_head(head: &[u8]) -> Result<(u16, HeaderMap, u8), Error> {
    parse_head(&Bytes::copy_from_slice(head))
}

pub(crate) fn parse_head(head: &Bytes) -> Result<(u16, HeaderMap, u8), Error> {
    let mut sink = Typed {
        head,
        fields: HeaderMap::with_capacity(memchr::memchr_iter(b'\n', head).count()),
        last: None,
    };
    let (status, minor) = parse(head, &mut sink)?;
    sink.flush();
    Ok((status, sink.fields, minor))
}

pub(crate) trait Sink<'a> {
    fn field(&mut self, name: &'a [u8], value: &'a [u8]) -> Result<(), Error>;
    fn fold(&mut self, line: &'a [u8]) -> Result<(), Error>;
}

pub(crate) struct Typed<'a> {
    head: &'a Bytes,
    fields: HeaderMap,
    last: Option<(http::HeaderName, http::HeaderValue)>,
}

impl Typed<'_> {
    fn flush(&mut self) {
        if let Some((name, value)) = self.last.take() {
            self.fields.append(name, value);
        }
    }
}

impl<'a> Sink<'a> for Typed<'_> {
    fn field(&mut self, name: &'a [u8], value: &'a [u8]) -> Result<(), Error> {
        self.flush();
        let value = self.head.slice_ref(trim_ows(value));
        let Ok(name) = http::HeaderName::from_bytes(name) else {
            return Ok(());
        };
        match http::HeaderValue::from_maybe_shared(value) {
            Ok(value) => {
                self.last = Some((name, value));
                Ok(())
            }
            Err(_) => framing_value(name.as_str()),
        }
    }

    fn fold(&mut self, line: &'a [u8]) -> Result<(), Error> {
        let Some((name, value)) = self.last.as_mut() else {
            return Ok(());
        };
        let mut joined = value.as_bytes().to_vec();
        joined.push(b' ');
        joined.extend_from_slice(trim_ows(line));
        if let Ok(folded) = http::HeaderValue::from_maybe_shared(Bytes::from(joined)) {
            *value = folded;
            return Ok(());
        }
        framing_value(name.as_str())?;
        self.last = None;
        Ok(())
    }
}

pub(crate) fn framing_value(name: &str) -> Result<(), Error> {
    if name == http::header::CONTENT_LENGTH || name == http::header::TRANSFER_ENCODING {
        return Err(malformed(format!(
            "response {name} value has a control character"
        )));
    }
    Ok(())
}

#[inline]
pub(crate) fn trim_start(mut bytes: &[u8]) -> &[u8] {
    while let Some(&b) = bytes.first() {
        if b == b' ' || b == b'\t' {
            bytes = &bytes[1..];
        } else {
            break;
        }
    }
    bytes
}

#[inline]
pub(crate) fn trim_end(mut bytes: &[u8]) -> &[u8] {
    while let Some((&b, rest)) = bytes.split_last() {
        if b == b' ' || b == b'\t' {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

#[inline]
fn next_crlf(bytes: &[u8]) -> Option<usize> {
    let mut offset = 0;
    while let Some(rel) = memchr::memchr(b'\n', &bytes[offset..]) {
        let i = offset + rel;
        if i > 0 && bytes[i - 1] == b'\r' {
            return Some(i - 1);
        }
        offset = i + 1;
    }
    None
}

pub(crate) fn status_line(line: &[u8]) -> Result<(u16, u8), Error> {
    if line.len() >= 12
        && &line[..7] == b"HTTP/1."
        && (line[7] == b'1' || line[7] == b'0')
        && line[8] == b' '
        && line[9].is_ascii_digit()
        && line[9] != b'0'
        && line[10].is_ascii_digit()
        && line[11].is_ascii_digit()
        && (line.len() == 12 || line[12] == b' ')
    {
        let status = u16::from(line[9] - b'0') * 100
            + u16::from(line[10] - b'0') * 10
            + u16::from(line[11] - b'0');
        return Ok((status, line[7] - b'0'));
    }
    status_line_slow(line)
}

#[cold]
fn status_line_slow(line: &[u8]) -> Result<(u16, u8), Error> {
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

pub(crate) fn parse<'a, S: Sink<'a>>(head: &'a [u8], sink: &mut S) -> Result<(u16, u8), Error> {
    let (first, mut rest) = match next_crlf(head) {
        Some(end) => (&head[..end], Some(&head[end + 2..])),
        None => (head, None),
    };
    let (status, minor) = status_line(first)?;
    while let Some(current) = rest {
        let line = match next_crlf(current) {
            Some(end) => {
                rest = Some(&current[end + 2..]);
                &current[..end]
            }
            None => {
                rest = None;
                current
            }
        };
        match line.first() {
            None => continue,
            Some(b' ' | b'\t') => {
                sink.fold(line)?;
                continue;
            }
            Some(_) => {}
        }
        let Some(colon) = memchr::memchr(b':', line) else {
            return Err(malformed(format!(
                "HTTP/1.1 header line has no colon: {}",
                String::from_utf8_lossy(line)
            )));
        };
        let name = &line[..colon];
        if !is_valid_token_bytes(name) {
            return Err(malformed(format!(
                "invalid HTTP/1.1 header name: {}",
                String::from_utf8_lossy(name)
            )));
        }
        sink.field(name, &line[colon + 1..])?;
    }
    Ok((status, minor))
}

pub(crate) fn chunk_size(line: &[u8]) -> Result<u64, String> {
    chunk_size_value(line).ok_or_else(|| {
        format!(
            "invalid chunk size line: {:?}",
            line.escape_ascii().to_string()
        )
    })
}

fn chunk_size_value(line: &[u8]) -> Option<u64> {
    let digits = line
        .iter()
        .take_while(|byte| byte.is_ascii_hexdigit())
        .count();
    let rest = &line[digits..];
    let blank = rest
        .iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count();
    if digits == 0 || rest.get(blank).is_some_and(|&byte| byte != b';') {
        return None;
    }
    line[..digits].iter().try_fold(0u64, |size, &byte| {
        let digit = char::from(byte).to_digit(16)?;
        size.checked_mul(16)?.checked_add(u64::from(digit))
    })
}

pub(crate) fn contains_header<K: AsRef<str>, V>(headers: &[(K, V)], name: &str) -> bool {
    crate::util::header_position(headers, name).is_some()
}

pub(crate) fn trim_ows(bytes: &[u8]) -> &[u8] {
    trim_end(trim_start(bytes))
}

pub(crate) fn is_valid_token_bytes(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(|&b| TOKEN[usize::from(b)])
}

pub(crate) fn is_valid_token(s: &str) -> bool {
    is_valid_token_bytes(s.as_bytes())
}

static TOKEN: [bool; 256] = {
    let mut table = [false; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = token_byte(i as u8);
        i += 1;
    }
    table
};

const fn token_byte(b: u8) -> bool {
    matches!(
        b,
        b'!' | b'#'
            | b'$'
            | b'%'
            | b'&'
            | b'\''
            | b'*'
            | b'+'
            | b'-'
            | b'.'
            | b'^'
            | b'_'
            | b'`'
            | b'|'
            | b'~'
            | b'0'..=b'9'
            | b'A'..=b'Z'
            | b'a'..=b'z'
    )
}

pub(crate) fn is_valid_header_value(s: impl AsRef<[u8]>) -> bool {
    s.as_ref().iter().fold(true, |valid, &b| {
        valid & ((b >= b' ') & (b != 0x7F) | (b == b'\t'))
    })
}

pub(crate) fn is_valid_request_target(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.bytes().all(|b| b > 0x20 && b != 0x7F)
}

pub(crate) fn h1_header_name(name: &str) -> &str {
    match name {
        _ if name.eq_ignore_ascii_case("host") => "Host",
        _ if name.eq_ignore_ascii_case("connection") => "Connection",
        _ if name.eq_ignore_ascii_case("user-agent") => "User-Agent",
        _ if name.eq_ignore_ascii_case("accept") => "Accept",
        _ if name.eq_ignore_ascii_case("accept-encoding") => "Accept-Encoding",
        _ if name.eq_ignore_ascii_case("accept-language") => "Accept-Language",
        _ if name.eq_ignore_ascii_case("content-length") => "Content-Length",
        _ if name.eq_ignore_ascii_case("content-type") => "Content-Type",
        _ if name.eq_ignore_ascii_case("cookie") => "Cookie",
        _ if name.eq_ignore_ascii_case("authorization") => "Authorization",
        _ if name.eq_ignore_ascii_case("proxy-authorization") => "Proxy-Authorization",
        _ if name.eq_ignore_ascii_case("origin") => "Origin",
        _ if name.eq_ignore_ascii_case("referer") => "Referer",
        _ if name.eq_ignore_ascii_case("upgrade") => "Upgrade",
        _ => name,
    }
}

pub(crate) enum HeaderEnd {
    Found(usize),
    Incomplete,
    BareLf,
}

#[inline(always)]
fn scan_header_end_inner(buf: &[u8], reject_bare_lf: bool) -> HeaderEnd {
    for i in memchr::memchr_iter(b'\n', buf) {
        if i == 0 || buf[i - 1] != b'\r' {
            if reject_bare_lf {
                return HeaderEnd::BareLf;
            }
            continue;
        }
        if i >= 3 && buf[i - 2] == b'\n' && buf[i - 3] == b'\r' {
            return HeaderEnd::Found(i - 3);
        }
    }
    HeaderEnd::Incomplete
}

pub(crate) fn scan_header_end(buf: &[u8]) -> HeaderEnd {
    scan_header_end_inner(buf, true)
}

pub(crate) fn find_header_end(buf: &[u8]) -> Option<usize> {
    match scan_header_end_inner(buf, false) {
        HeaderEnd::Found(pos) => Some(pos),
        HeaderEnd::Incomplete | HeaderEnd::BareLf => None,
    }
}

pub(crate) fn put(req: &mut Vec<u8>, name: &str, value: &str) {
    req.extend_from_slice(name.as_bytes());
    req.extend_from_slice(b": ");
    req.extend_from_slice(value.as_bytes());
    req.extend_from_slice(b"\r\n");
}

pub(crate) fn field_lines(headers: &[HeaderPair]) -> Option<Vec<u8>> {
    let size = headers
        .iter()
        .map(|(name, value)| name.len() + value.len() + 4)
        .sum();
    let mut lines = Vec::with_capacity(size);
    for (name, value) in headers {
        if !crate::wire::sent_on(http::Version::HTTP_11, name, value) {
            continue;
        }
        if !is_valid_token(name) || !is_valid_header_value(value) {
            return None;
        }
        put(&mut lines, h1_header_name(name), value);
    }
    Some(lines)
}

#[cfg(kani)]
#[path = "chunk_proofs.rs"]
mod chunk_proofs;

#[cfg(test)]
mod chunk_tests;
