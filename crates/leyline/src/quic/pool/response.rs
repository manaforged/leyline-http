use bytes::Bytes;
use leyline_quiche as quiche;
use quiche::h3::NameValue;

use http::HeaderMap;

use crate::core::transport::ResponseHead;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum H3ResponseState {
    Initial(usize),
    Final,
    Trailers,
}

pub(super) enum H3HeaderBlock {
    Informational,
    Final {
        status: http::StatusCode,
        headers: http::HeaderMap,
    },
    Trailers(http::HeaderMap),
}

impl H3ResponseState {
    pub(super) fn headers(
        &mut self,
        list: &[quiche::h3::Header],
    ) -> Result<H3HeaderBlock, &'static str> {
        match self {
            Self::Initial(informational) => {
                let (status, headers) = response_head(list)?;
                if status.is_informational() {
                    *informational += 1;
                    if *informational > crate::util::MAX_INFORMATIONAL {
                        return Err("h3: too many informational responses");
                    }
                    Ok(H3HeaderBlock::Informational)
                } else {
                    *self = Self::Final;
                    Ok(H3HeaderBlock::Final { status, headers })
                }
            }
            Self::Final => {
                if list.iter().any(|header| header.name().starts_with(b":")) {
                    return Err("h3: trailers must not contain pseudo-headers");
                }
                *self = Self::Trailers;
                Ok(H3HeaderBlock::Trailers(
                    list.iter().filter_map(header_pair).collect(),
                ))
            }
            Self::Trailers => Err("h3: response contains headers after trailers"),
        }
    }

    pub(super) fn data(self) -> Result<(), &'static str> {
        match self {
            Self::Final => Ok(()),
            Self::Initial(_) => Err("h3: response DATA arrived before a final response head"),
            Self::Trailers => Err("h3: response DATA arrived after trailers"),
        }
    }

    pub(super) fn finish(self) -> Result<(), &'static str> {
        match self {
            Self::Initial(_) => Err("h3: response ended before a final response head"),
            Self::Final | Self::Trailers => Ok(()),
        }
    }
}

fn header_pair(header: &quiche::h3::Header) -> Option<(http::HeaderName, http::HeaderValue)> {
    Some((
        http::HeaderName::from_bytes(header.name()).ok()?,
        http::HeaderValue::from_maybe_shared(Bytes::copy_from_slice(header.value())).ok()?,
    ))
}

fn response_head(
    list: &[quiche::h3::Header],
) -> Result<(http::StatusCode, HeaderMap), &'static str> {
    let mut head = ResponseHead::default();
    for header in list {
        if header.name().starts_with(b":") {
            head.pseudo(header.name(), header.value())?;
        } else {
            head.field(header_pair(header));
        }
    }
    head.finish()
}
