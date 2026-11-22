use std::fmt;
use std::sync::OnceLock;

use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue};

use super::h1::{find_values, parse_head};
use crate::util::has_token;

/// Header facts the request path reads without a header map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Seen {
    pub(crate) cookies: bool,
    pub(crate) hsts: bool,
    pub(crate) alt_svc: bool,
    pub(crate) encoded: bool,
    pub(crate) close: bool,
    pub(crate) keep_alive: bool,
}

/// A response field whose presence alone sets one `Seen` flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fact {
    Cookies,
    Hsts,
    AltSvc,
    Encoded,
}

impl Fact {
    const ALL: [Self; 4] = [Self::Cookies, Self::Hsts, Self::AltSvc, Self::Encoded];

    fn header(self) -> HeaderName {
        match self {
            Self::Cookies => http::header::SET_COOKIE,
            Self::Hsts => http::header::STRICT_TRANSPORT_SECURITY,
            Self::AltSvc => http::header::ALT_SVC,
            Self::Encoded => http::header::CONTENT_ENCODING,
        }
    }

    pub(crate) fn from_header(name: &HeaderName) -> Option<Self> {
        Self::ALL.into_iter().find(|fact| *name == fact.header())
    }

    pub(crate) fn named(name: &[u8]) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|fact| name.eq_ignore_ascii_case(fact.header().as_str().as_bytes()))
    }
}

impl Seen {
    pub(crate) fn note_header(&mut self, name: &HeaderName) {
        if let Some(fact) = Fact::from_header(name) {
            self.note(fact);
        }
    }

    pub(crate) fn note(&mut self, fact: Fact) {
        match fact {
            Fact::Cookies => self.cookies = true,
            Fact::Hsts => self.hsts = true,
            Fact::AltSvc => self.alt_svc = true,
            Fact::Encoded => self.encoded = true,
        }
    }

    pub(crate) fn note_connection(&mut self, value: &str) {
        self.close |= has_token(value, "close");
        self.keep_alive |= has_token(value, "keep-alive");
    }

    pub(crate) fn note_connection_bytes(&mut self, value: &[u8]) {
        if let Some(v) = text(value) {
            self.note_connection(v);
        }
    }

    fn of(map: &HeaderMap) -> Self {
        let mut seen = Self::default();
        let mut connection = false;
        for name in map.keys() {
            seen.note_header(name);
            connection |= name == http::header::CONNECTION;
        }
        if !connection {
            return seen;
        }
        for value in map.get_all(http::header::CONNECTION) {
            if let Ok(v) = value.to_str() {
                seen.note_connection(v);
            }
        }
        seen
    }
}

/// Response header fields, parsed into a map on first read.
pub struct Fields(Repr);

enum Repr {
    Map(HeaderMap),
    Raw {
        head: Bytes,
        seen: Seen,
        map: OnceLock<Box<HeaderMap>>,
    },
    List {
        pairs: Vec<(HeaderName, HeaderValue)>,
        seen: Seen,
        map: OnceLock<Box<HeaderMap>>,
    },
}

impl Fields {
    pub(crate) fn raw(head: Bytes, seen: Seen) -> Self {
        Self(Repr::Raw {
            head,
            seen,
            map: OnceLock::new(),
        })
    }

    pub(crate) fn list(pairs: Vec<(HeaderName, HeaderValue)>, seen: Seen) -> Self {
        Self(Repr::List {
            pairs,
            seen,
            map: OnceLock::new(),
        })
    }

    /// Returns the fields as a header map, parsing them on the first call.
    pub fn map(&self) -> &HeaderMap {
        match &self.0 {
            Repr::Map(map) => map,
            Repr::Raw { head, map, .. } => map.get_or_init(|| Box::new(build(head))),
            Repr::List { pairs, map, .. } => map.get_or_init(|| {
                Box::new(pairs_to_map(
                    pairs.iter().map(|(n, v)| (n.clone(), v.clone())),
                    pairs.len(),
                ))
            }),
        }
    }

    /// Returns the fields as an owned header map.
    pub fn into_map(self) -> HeaderMap {
        match self.0 {
            Repr::Map(map) => map,
            Repr::Raw { head, map, .. } => {
                map.into_inner().map_or_else(|| build(&head), |map| *map)
            }
            Repr::List { pairs, map, .. } => map.into_inner().map_or_else(
                || {
                    let len = pairs.len();
                    pairs_to_map(pairs.into_iter(), len)
                },
                |map| *map,
            ),
        }
    }

    /// Iterates over the fields in order.
    #[cfg(feature = "bench-internals")]
    pub fn iter(&self) -> http::header::Iter<'_, http::HeaderValue> {
        self.map().iter()
    }

    pub(crate) fn seen(&self) -> Seen {
        match &self.0 {
            Repr::Map(map) => Seen::of(map),
            Repr::Raw { seen, .. } | Repr::List { seen, .. } => *seen,
        }
    }

    pub(crate) fn any_value(&self, name: &HeaderName, mut hit: impl FnMut(&[u8]) -> bool) -> bool {
        match &self.0 {
            Repr::Raw { head, map, .. } if map.get().is_none() => find_values(head, name, hit),
            Repr::List { pairs, map, .. } if map.get().is_none() => pairs
                .iter()
                .filter(|(k, _)| k == name)
                .any(|(_, v)| hit(v.as_bytes())),
            _ => self.map().get_all(name).iter().any(|v| hit(v.as_bytes())),
        }
    }
}

fn pairs_to_map(
    pairs: impl Iterator<Item = (HeaderName, HeaderValue)>,
    capacity: usize,
) -> HeaderMap {
    let mut out = HeaderMap::with_capacity(capacity);
    for (name, value) in pairs {
        out.append(name, value);
    }
    out
}

fn build(head: &Bytes) -> HeaderMap {
    parse_head(head).map(|(_, map, _)| map).unwrap_or_default()
}

pub(crate) fn text(value: &[u8]) -> Option<&str> {
    value
        .iter()
        .all(|&b| b == b'\t' || (32..127).contains(&b))
        .then(|| std::str::from_utf8(value).ok())
        .flatten()
}

impl Default for Fields {
    fn default() -> Self {
        Self(Repr::Map(HeaderMap::new()))
    }
}

impl From<HeaderMap> for Fields {
    fn from(map: HeaderMap) -> Self {
        Self(Repr::Map(map))
    }
}

impl fmt::Debug for Fields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.map(), f)
    }
}
