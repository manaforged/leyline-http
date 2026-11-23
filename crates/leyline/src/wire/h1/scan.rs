use std::borrow::Cow;

use http::HeaderName;

use super::*;
use crate::wire::fields::Fact;
use crate::wire::text;

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
        let value = trim_ows(value);
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
        let line = trim_ows(line);
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
    Fact(Fact),
}

impl Known {
    fn name(self) -> Option<&'static str> {
        match self {
            Self::Length => Some("content-length"),
            Self::Coding => Some("transfer-encoding"),
            Self::Connection => Some("connection"),
            Self::Fact(_) => None,
        }
    }
}

#[derive(Default)]
pub(crate) struct Scanned<'a> {
    pub(crate) body: BodyHeaders<'a>,
    seen: Seen,
}

impl Scanned<'_> {
    pub(crate) fn seen(&self) -> Seen {
        self.seen
    }
}

impl<'a> Pick<'a> for Scanned<'a> {
    type Key = Known;

    #[inline]
    fn key(&self, name: &[u8]) -> Option<Known> {
        match name.len() {
            10 if name.eq_ignore_ascii_case(b"connection") => Some(Known::Connection),
            14 if name.eq_ignore_ascii_case(b"content-length") => Some(Known::Length),
            17 if name.eq_ignore_ascii_case(b"transfer-encoding") => Some(Known::Coding),
            _ => Fact::named(name).map(Known::Fact),
        }
    }

    fn framing(key: Known) -> Option<&'static str> {
        matches!(key, Known::Length | Known::Coding)
            .then(|| key.name())
            .flatten()
    }

    fn take(&mut self, key: Known, value: Cow<'a, [u8]>) {
        match key {
            Known::Length => {
                self.body.lengths += 1;
                self.body.length.get_or_insert(value);
            }
            Known::Coding => match &mut self.body.coding {
                Some(coding) => *coding = Cow::Owned([&**coding, b",", &*value].concat()),
                None => self.body.coding = Some(value),
            },
            Known::Connection => self.seen.note_connection_bytes(&value),
            Known::Fact(fact) => self.seen.note(fact),
        }
    }
}

pub(crate) fn scan_head(head: &[u8]) -> Result<(u16, u8, Scanned<'_>), Error> {
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
pub(crate) struct BodyHeaders<'a> {
    lengths: usize,
    length: Option<Cow<'a, [u8]>>,
    coding: Option<Cow<'a, [u8]>>,
}

enum FramingError<'a> {
    MultipleLength,
    NonAsciiLength,
    CommaLength,
    BadLength(&'a str),
    BothLengthAndCoding,
    NonAsciiCoding,
    BadCoding(&'a str),
}

impl BodyHeaders<'_> {
    fn length(&self) -> Option<&str> {
        self.length.as_deref().and_then(text)
    }

    fn coding(&self) -> Option<&str> {
        self.coding.as_deref().and_then(text)
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.check_framing().map_err(|err| match err {
            FramingError::MultipleLength => {
                malformed("response has multiple Content-Length headers (RFC 9112 §6.1)")
            }
            FramingError::NonAsciiLength => {
                malformed("response Content-Length contains non-ASCII bytes (RFC 9112 §8.6)")
            }
            FramingError::CommaLength => {
                malformed("response Content-Length contains multiple values")
            }
            FramingError::BadLength(v) => malformed(format!(
                "response Content-Length `{v}` is not a valid decimal integer (RFC 9112 §8.6)"
            )),
            FramingError::BothLengthAndCoding => {
                malformed("response has both Content-Length and Transfer-Encoding (RFC 9112 §6.1)")
            }
            FramingError::NonAsciiCoding => {
                malformed("response Transfer-Encoding contains non-ASCII bytes")
            }
            FramingError::BadCoding(te) => malformed(format!(
                "response Transfer-Encoding `{te}`: `chunked` must be the final coding, once"
            )),
        })
    }

    fn check_framing(&self) -> Result<(), FramingError<'_>> {
        if self.lengths > 1 {
            return Err(FramingError::MultipleLength);
        }
        if self.length.is_some() {
            let Some(v) = self.length() else {
                return Err(FramingError::NonAsciiLength);
            };
            if v.as_bytes().contains(&b',') {
                return Err(FramingError::CommaLength);
            }
            if crate::util::content_length(v).is_none() {
                return Err(FramingError::BadLength(v));
            }
        }
        if self.coding.is_some() && self.lengths > 0 {
            return Err(FramingError::BothLengthAndCoding);
        }
        if self.coding.is_some() {
            let Some(te) = self.coding() else {
                return Err(FramingError::NonAsciiCoding);
            };
            if !is_single_final_chunked(te.as_bytes()) {
                return Err(FramingError::BadCoding(te));
            }
        }
        Ok(())
    }

    pub(crate) fn framing(&self) -> BodyFraming {
        if self.coding().is_some_and(|te| has_token(te, "chunked")) {
            BodyFraming::Chunked
        } else if let Some(len) = self.length().and_then(crate::util::content_length) {
            BodyFraming::Fixed(len)
        } else {
            BodyFraming::ToClose
        }
    }
}

fn is_single_final_chunked(bytes: &[u8]) -> bool {
    let mut last_is_chunked = false;
    let mut chunked = 0usize;
    crate::util::any_token(bytes, |tok| {
        last_is_chunked = tok.eq_ignore_ascii_case(b"chunked");
        if last_is_chunked {
            chunked += 1;
        }
        false
    });
    chunked == 1 && last_is_chunked
}

#[cfg(test)]
#[path = "scan_framing_tests.rs"]
mod framing_tests;

#[cfg(kani)]
#[path = "scan_proofs.rs"]
mod proofs;
