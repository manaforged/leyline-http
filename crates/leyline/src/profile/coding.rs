#[cfg(not(leyline_all_codecs))]
use crate::error::{Error, Kind};

/// A content encoding for compressing a request body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Encoding {
    /// The gzip encoding.
    Gzip,
    /// The Brotli encoding.
    Brotli,
    /// The Zstandard encoding.
    Zstd,
    /// The deflate encoding.
    Deflate,
}

impl Encoding {
    /// Every coding, in `Accept-Encoding` order; the index is the coding's bit in an offer mask.
    pub(crate) const ALL: [Self; 4] = [Self::Gzip, Self::Deflate, Self::Brotli, Self::Zstd];

    /// The token of the identity coding, which leaves the body as sent.
    pub(crate) const IDENTITY: &'static str = "identity";

    /// Parses a `Content-Encoding` token, with `x-gzip` as an alias of gzip.
    pub(crate) fn parse(token: &str) -> Option<Self> {
        match token {
            "gzip" | "x-gzip" => Some(Self::Gzip),
            "deflate" => Some(Self::Deflate),
            "br" => Some(Self::Brotli),
            "zstd" => Some(Self::Zstd),
            _ => None,
        }
    }

    /// Returns the token sent in `Content-Encoding` and `Accept-Encoding`.
    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::Gzip => "gzip",
            Self::Deflate => "deflate",
            Self::Brotli => "br",
            Self::Zstd => "zstd",
        }
    }

    /// Returns the name used in error messages.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Brotli => "brotli",
            other => other.token(),
        }
    }

    /// Returns the Cargo feature that compiles this coding in.
    #[cfg(not(leyline_all_codecs))]
    pub(crate) fn feature(self) -> &'static str {
        match self {
            Self::Gzip => "compression-gzip",
            Self::Deflate => "compression-deflate",
            Self::Brotli => "compression-brotli",
            Self::Zstd => "compression-zstd",
        }
    }

    /// Reports whether this coding's feature is compiled in.
    pub(crate) fn compiled(self) -> bool {
        match self {
            Self::Gzip => cfg!(feature = "compression-gzip"),
            Self::Deflate => cfg!(feature = "compression-deflate"),
            Self::Brotli => cfg!(feature = "compression-brotli"),
            Self::Zstd => cfg!(feature = "compression-zstd"),
        }
    }

    /// Returns the error for `action` on this coding when its feature is not compiled in.
    #[cfg(not(leyline_all_codecs))]
    pub(crate) fn feature_off(self, kind: Kind, action: &str) -> Error {
        Error::new(kind).with_message(format!(
            "{} {action} but the `{}` feature is not compiled in",
            self.name(),
            self.feature()
        ))
    }

    /// Returns this coding's bit in an offer mask: one shifted by its index in `ALL`.
    pub(crate) fn bit(self) -> u8 {
        match self {
            Self::Gzip => 1,
            Self::Deflate => 1 << 1,
            Self::Brotli => 1 << 2,
            Self::Zstd => 1 << 3,
        }
    }
}
