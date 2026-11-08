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
        let value = self.head.slice_ref(trim_start(value));
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
        joined.extend_from_slice(trim_end(trim_start(line)));
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

pub(crate) fn trim_start(mut bytes: &[u8]) -> &[u8] {
    while let Some(n) = edge_space(bytes, true) {
        bytes = &bytes[n..];
    }
    bytes
}

pub(crate) fn trim_end(mut bytes: &[u8]) -> &[u8] {
    while let Some(n) = edge_space(bytes, false) {
        bytes = &bytes[..bytes.len() - n];
    }
    bytes
}

pub(crate) fn edge_space(bytes: &[u8], front: bool) -> Option<usize> {
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

pub(crate) fn lines(head: &[u8]) -> impl Iterator<Item = &[u8]> {
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

pub(crate) fn status_line(line: &[u8]) -> Result<(u16, u8), Error> {
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
    headers
        .iter()
        .any(|(k, _)| k.as_ref().eq_ignore_ascii_case(name))
}

pub(crate) fn is_valid_token(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| TOKEN[usize::from(b)])
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

pub(crate) fn find_header_end(buf: &[u8]) -> Option<usize> {
    memchr::memchr_iter(b'\n', buf)
        .find(|&i| i >= 3 && &buf[i - 3..=i] == b"\r\n\r\n")
        .map(|i| i - 3)
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
