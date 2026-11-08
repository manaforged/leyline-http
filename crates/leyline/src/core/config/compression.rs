use std::sync::LazyLock;

use crate::profile::Encoding;

/// The most content codings one response may list and still be decoded.
pub(crate) const MAX_CODINGS: usize = 4;

/// The `Content-Encoding` of a response, parsed once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Codings {
    /// The response lists no content coding.
    Absent,
    /// The listed codings in header order, without identity entries.
    Listed(Vec<Encoding>),
    /// The response lists a coding the crate does not know.
    Unknown,
    /// The response lists this many codings, more than `MAX_CODINGS`.
    TooMany(usize),
}

impl Codings {
    /// Parses every `Content-Encoding` field value of a response.
    pub(crate) fn of<'a>(values: impl IntoIterator<Item = &'a http::HeaderValue>) -> Self {
        let mut listed = Vec::new();
        let mut count = 0usize;
        let mut unknown = false;
        let mut any = false;
        for value in values {
            any = true;
            let value = String::from_utf8_lossy(value.as_bytes()).to_ascii_lowercase();
            for token in value.split(',').map(str::trim) {
                count += 1;
                if token.is_empty() || token == Encoding::IDENTITY {
                    continue;
                }
                match Encoding::parse(token) {
                    Some(coding) => listed.push(coding),
                    None => unknown = true,
                }
            }
        }
        if !any {
            Self::Absent
        } else if count > MAX_CODINGS {
            Self::TooMany(count)
        } else if unknown {
            Self::Unknown
        } else {
            Self::Listed(listed)
        }
    }

    /// Reports whether the body is sent as is: no coding, or only identity.
    pub(crate) fn is_identity(&self) -> bool {
        match self {
            Self::Absent => true,
            Self::Listed(listed) => listed.is_empty(),
            Self::Unknown | Self::TooMany(_) => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BodyRules {
    offered: u8,
    pub(crate) decode: bool,
    pub(crate) max_body_size: usize,
    pub(crate) max_error_body: usize,
}

impl Default for BodyRules {
    fn default() -> Self {
        Self {
            offered: Encoding::ALL
                .iter()
                .fold(0, |mask, coding| mask | coding.bit()),
            decode: true,
            max_body_size: crate::wire::DEFAULT_MAX_BODY_SIZE,
            max_error_body: 64 * 1024,
        }
    }
}

impl BodyRules {
    pub(crate) fn offer(&mut self, codings: impl IntoIterator<Item = Encoding>) {
        self.offered = codings
            .into_iter()
            .fold(0, |mask, coding| mask | coding.bit());
    }

    pub(crate) fn accept_encoding(&self) -> Option<&'static str> {
        let mask = Encoding::ALL
            .iter()
            .filter(|coding| coding.compiled())
            .fold(0, |mask, coding| mask | coding.bit())
            & self.offered;
        (mask != 0).then(|| CODEC_LISTS[usize::from(mask)].as_str())
    }

    pub(crate) fn allows(&self, coding: Encoding) -> bool {
        self.decode && self.offered & coding.bit() != 0
    }
}

const OFFER_MASKS: usize = 1 << Encoding::ALL.len();

static CODEC_LISTS: LazyLock<[String; OFFER_MASKS]> = LazyLock::new(|| {
    std::array::from_fn(|mask| {
        Encoding::ALL
            .iter()
            .filter(|coding| mask & usize::from(coding.bit()) != 0)
            .map(|coding| coding.token())
            .collect::<Vec<_>>()
            .join(", ")
    })
});
