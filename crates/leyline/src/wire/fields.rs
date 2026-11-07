use std::fmt;
use std::sync::OnceLock;

use bytes::Bytes;
use http::{HeaderMap, HeaderName};

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

    pub(crate) fn named(name: &[u8]) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|fact| name.eq_ignore_ascii_case(fact.header().as_str().as_bytes()))
    }
}

impl Seen {
    pub(crate) fn note(&mut self, fact: Fact) {
        match fact {
            Fact::Cookies => self.cookies = true,
            Fact::Hsts => self.hsts = true,
            Fact::AltSvc => self.alt_svc = true,
            Fact::Encoded => self.encoded = true,
        }
    }

    fn of(map: &HeaderMap) -> Self {
        let mut seen = Self::default();
        let mut connection = false;
        for name in map.keys() {
            if let Some(fact) = Fact::ALL.into_iter().find(|fact| *name == fact.header()) {
                seen.note(fact);
            }
            connection |= name == http::header::CONNECTION;
        }
        if !connection {
            return seen;
        }
        seen.connection(
            map.get(http::header::CONNECTION)
                .and_then(|v| v.to_str().ok()),
        )
    }

    pub(crate) fn connection(mut self, value: Option<&str>) -> Self {
        self.close = value.is_some_and(|v| has_token(v, "close"));
        self.keep_alive = value.is_some_and(|v| has_token(v, "keep-alive"));
        self
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
}

impl Fields {
    pub(crate) fn raw(head: Bytes, seen: Seen) -> Self {
        Self(Repr::Raw {
            head,
            seen,
            map: OnceLock::new(),
        })
    }

    /// Returns the fields as a header map, parsing them on the first call.
    pub fn map(&self) -> &HeaderMap {
        match &self.0 {
            Repr::Map(map) => map,
            Repr::Raw { head, map, .. } => map.get_or_init(|| Box::new(build(head))),
        }
    }

    /// Returns the fields as an owned header map.
    pub fn into_map(self) -> HeaderMap {
        match self.0 {
            Repr::Map(map) => map,
            Repr::Raw { head, map, .. } => {
                map.into_inner().map_or_else(|| build(&head), |map| *map)
            }
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
            Repr::Raw { seen, .. } => *seen,
        }
    }

    pub(crate) fn any_value(&self, name: &HeaderName, mut hit: impl FnMut(&[u8]) -> bool) -> bool {
        match &self.0 {
            Repr::Raw { head, map, .. } if map.get().is_none() => find_values(head, name, hit),
            _ => self.map().get_all(name).iter().any(|v| hit(v.as_bytes())),
        }
    }
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
