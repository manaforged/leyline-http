use bytes::Bytes;
use leyline_quiche as quiche;
use quiche::h3::NameValue;

use http::HeaderMap;

use crate::core::transport::{ResponseHead, ResponseState, response_field};

pub(super) enum H3HeaderBlock {
    Informational,
    Final {
        status: http::StatusCode,
        headers: http::HeaderMap,
        length: Option<u64>,
    },
    Trailers(http::HeaderMap),
}

impl ResponseState {
    pub(super) fn headers(
        &mut self,
        list: &[quiche::h3::Header],
    ) -> Result<H3HeaderBlock, &'static str> {
        match self {
            Self::Initial(_) => {
                let (status, headers, length) = response_head(list)?;
                self.head(status)?;
                if status.is_informational() {
                    Ok(H3HeaderBlock::Informational)
                } else {
                    Ok(H3HeaderBlock::Final {
                        status,
                        headers,
                        length,
                    })
                }
            }
            Self::Final => {
                if list.iter().any(|header| header.name().starts_with(b":")) {
                    return Err("h3: trailers must not contain pseudo-headers");
                }
                self.trailers()?;
                Ok(H3HeaderBlock::Trailers(
                    list.iter().map(header_pair).collect::<Result<_, _>>()?,
                ))
            }
            Self::Trailers => Err("h3: response contains headers after trailers"),
        }
    }
}

fn header_pair(
    header: &quiche::h3::Header,
) -> Result<(http::HeaderName, http::HeaderValue), &'static str> {
    let typed = http::HeaderName::from_bytes(header.name())
        .ok()
        .zip(http::HeaderValue::from_maybe_shared(Bytes::copy_from_slice(header.value())).ok());
    response_field(header.name(), header.value(), typed)
}

fn response_head(
    list: &[quiche::h3::Header],
) -> Result<(http::StatusCode, HeaderMap, Option<u64>), &'static str> {
    let mut head = ResponseHead::default();
    for header in list {
        if header.name().starts_with(b":") {
            head.pseudo(header.name(), header.value())?;
        } else {
            head.field(header_pair(header))?;
        }
    }
    head.finish()
}
