use std::borrow::Cow;

use http::HeaderName;

use super::*;
use crate::core::response_head::text;

trait Pick<'a> {
    type Key: Copy;
    fn key(&self, name: &[u8]) -> Option<Self::Key>;
    fn framing(key: Self::Key) -> Option<&'static str>;
    fn take(&mut self, key: Self::Key, value: Cow<'a, [u8]>);
}

struct Fold<'a, P: Pick<'a>> {
    pick: P,
    last: Option<(P::Key, Cow<'a, [u8]>)>,
}

impl<'a, P: Pick<'a>> Fold<'a, P> {
    fn run(head: &'a [u8], pick: P) -> Result<(u16, u8, P), Error> {
        let mut sink = Fold { pick, last: None };
        let (status, minor) = parse(head, &mut sink)?;
        sink.flush();
        Ok((status, minor, sink.pick))
    }

    fn flush(&mut self) {
        if let Some((key, value)) = self.last.take() {
            self.pick.take(key, value);
        }
    }

    fn invalid(key: P::Key) -> Result<(), Error> {
        P::framing(key).map_or(Ok(()), framing_value)
    }
}

impl<'a, P: Pick<'a>> Sink<'a> for Fold<'a, P> {
    fn field(&mut self, name: &'a [u8], value: &'a [u8]) -> Result<(), Error> {
        self.flush();
        let Some(key) = self.pick.key(name) else {
            return Ok(());
        };
        let value = trim_start(value);
        if !is_valid_header_value(value) {
            return Self::invalid(key);
        }
        self.last = Some((key, Cow::Borrowed(value)));
        Ok(())
    }

    fn fold(&mut self, line: &'a [u8]) -> Result<(), Error> {
        let Some((key, value)) = self.last.as_mut() else {
            return Ok(());
        };
        let line = trim_end(trim_start(line));
        if is_valid_header_value(line) {
            let joined = value.to_mut();
            joined.push(b' ');
            joined.extend_from_slice(line);
            return Ok(());
        }
        let key = *key;
        Self::invalid(key)?;
        self.last = None;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Known {
    Length,
    Coding,
    Connection,
    Encoding,
    Cookie,
    Hsts,
    AltSvc,
}

impl Known {
    const ALL: [Self; 7] = [
        Self::Length,
        Self::Coding,
        Self::Connection,
        Self::Encoding,
        Self::Cookie,
        Self::Hsts,
        Self::AltSvc,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Length => "content-length",
            Self::Coding => "transfer-encoding",
            Self::Connection => "connection",
            Self::Encoding => "content-encoding",
            Self::Cookie => "set-cookie",
            Self::Hsts => "strict-transport-security",
            Self::AltSvc => "alt-svc",
        }
    }
}

#[derive(Default)]
pub(super) struct Scanned<'a> {
    pub(super) body: BodyHeaders<'a>,
    connection: Option<Cow<'a, [u8]>>,
    seen: Seen,
}

impl Scanned<'_> {
    pub(super) fn seen(&self) -> Seen {
        self.seen
            .connection(self.connection.as_deref().and_then(text))
    }
}

impl<'a> Pick<'a> for Scanned<'a> {
    type Key = Known;

    fn key(&self, name: &[u8]) -> Option<Known> {
        Known::ALL
            .into_iter()
            .find(|known| name.eq_ignore_ascii_case(known.name().as_bytes()))
    }

    fn framing(key: Known) -> Option<&'static str> {
        matches!(key, Known::Length | Known::Coding).then(|| key.name())
    }

    fn take(&mut self, key: Known, value: Cow<'a, [u8]>) {
        match key {
            Known::Length => {
                self.body.lengths += 1;
                self.body.length.get_or_insert(value);
            }
            Known::Coding => {
                self.body.coding.get_or_insert(value);
            }
            Known::Connection => {
                self.connection.get_or_insert(value);
            }
            Known::Encoding => self.seen.encoded = true,
            Known::Cookie => self.seen.cookies = true,
            Known::Hsts => self.seen.hsts = true,
            Known::AltSvc => self.seen.alt_svc = true,
        }
    }
}

pub(super) fn scan_head(head: &[u8]) -> Result<(u16, u8, Scanned<'_>), Error> {
    Fold::run(head, Scanned::default())
}

struct Target<'n, F> {
    name: &'n HeaderName,
    hit: F,
    found: bool,
}

impl<'a, F: FnMut(&[u8]) -> bool> Pick<'a> for Target<'_, F> {
    type Key = ();

    fn key(&self, name: &[u8]) -> Option<()> {
        name.eq_ignore_ascii_case(self.name.as_str().as_bytes())
            .then_some(())
    }

    fn framing((): ()) -> Option<&'static str> {
        None
    }

    fn take(&mut self, (): (), value: Cow<'a, [u8]>) {
        if !self.found {
            self.found = (self.hit)(&value);
        }
    }
}

pub(crate) fn find_values(head: &[u8], name: &HeaderName, hit: impl FnMut(&[u8]) -> bool) -> bool {
    let wanted = name.as_str().as_bytes();
    let named = std::iter::once(0)
        .chain(memchr::memchr_iter(b'\n', head).map(|end| end + 1))
        .any(|start| {
            head.get(start..start + wanted.len())
                .is_some_and(|line| line.eq_ignore_ascii_case(wanted))
        });
    if !named {
        return false;
    }
    let target = Target {
        name,
        hit,
        found: false,
    };
    Fold::run(head, target).is_ok_and(|(_, _, target)| target.found)
}

#[derive(Default)]
pub(super) struct BodyHeaders<'a> {
    lengths: usize,
    length: Option<Cow<'a, [u8]>>,
    coding: Option<Cow<'a, [u8]>>,
}

impl BodyHeaders<'_> {
    fn length(&self) -> Option<&str> {
        self.length.as_deref().and_then(text)
    }

    fn coding(&self) -> Option<&str> {
        self.coding.as_deref().and_then(text)
    }

    pub(super) fn validate(&self) -> Result<(), Error> {
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

    pub(super) fn framing(&self) -> BodyFraming {
        if self.coding().is_some_and(|te| has_token(te, "chunked")) {
            BodyFraming::Chunked
        } else if let Some(len) = self.length().and_then(crate::util::content_length) {
            BodyFraming::Fixed(len)
        } else {
            BodyFraming::ToClose
        }
    }
}
