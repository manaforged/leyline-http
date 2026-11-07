use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};

/// The most informational (1xx) responses leyline reads before the final one.
pub(crate) const MAX_INFORMATIONAL: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResponseState {
    Initial(usize),
    Final,
    Trailers,
}

impl Default for ResponseState {
    fn default() -> Self {
        Self::Initial(0)
    }
}

impl ResponseState {
    pub(crate) fn head(&mut self, status: StatusCode) -> Result<(), &'static str> {
        let Self::Initial(informational) = self else {
            return Err("response head arrived after a final response head");
        };
        if status == StatusCode::SWITCHING_PROTOCOLS {
            return Err("status 101 is forbidden");
        }
        if status.is_informational() {
            if *informational >= MAX_INFORMATIONAL {
                return Err("too many informational responses");
            }
            *informational += 1;
        } else {
            *self = Self::Final;
        }
        Ok(())
    }

    pub(crate) fn trailers(&mut self) -> Result<(), &'static str> {
        match self {
            Self::Final => {
                *self = Self::Trailers;
                Ok(())
            }
            Self::Initial(_) => Err("response trailers arrived before a final response head"),
            Self::Trailers => Err("response contains headers after trailers"),
        }
    }

    pub(crate) fn data(self) -> Result<(), &'static str> {
        match self {
            Self::Final => Ok(()),
            Self::Initial(_) => Err("response DATA arrived before a final response head"),
            Self::Trailers => Err("response DATA arrived after trailers"),
        }
    }

    pub(crate) fn finish(self) -> Result<(), &'static str> {
        match self {
            Self::Initial(_) => Err("response ended before a final response head"),
            Self::Final | Self::Trailers => Ok(()),
        }
    }
}

/// One response header block as h2 and h3 deliver it.
pub(crate) enum ResponseBlock {
    Informational,
    Final {
        status: StatusCode,
        headers: HeaderMap,
        length: Option<u64>,
    },
    Trailers(HeaderMap),
}

enum Collect {
    Head(ResponseHead),
    Trailers(HeaderMap),
}

/// Collects one h2 or h3 response header block field by field.
pub(crate) struct BlockReader {
    collect: Collect,
    error: Option<&'static str>,
}

impl BlockReader {
    pub(crate) fn new(state: ResponseState) -> Self {
        let collect = match state {
            ResponseState::Initial(_) => Collect::Head(ResponseHead::default()),
            ResponseState::Final | ResponseState::Trailers => Collect::Trailers(HeaderMap::new()),
        };
        Self {
            collect,
            error: None,
        }
    }

    pub(crate) fn field(
        &mut self,
        name: &[u8],
        value: &[u8],
        typed: impl FnOnce() -> Option<(HeaderName, HeaderValue)>,
    ) {
        if self.error.is_some() {
            return;
        }
        let pseudo = name.starts_with(b":");
        let result = match &mut self.collect {
            Collect::Head(head) if pseudo => head.pseudo(name, value),
            Collect::Head(head) => head.field(response_field(name, value, typed())),
            Collect::Trailers(_) if pseudo => Err("trailers must not contain pseudo-headers"),
            Collect::Trailers(fields) => {
                response_field(name, value, typed()).map(|(name, value)| {
                    fields.append(name, value);
                })
            }
        };
        self.error = result.err();
    }

    pub(crate) fn finish(self, state: &mut ResponseState) -> Result<ResponseBlock, &'static str> {
        match self.collect {
            Collect::Head(head) => {
                if let Some(error) = self.error {
                    return Err(error);
                }
                let (status, headers, length) = head.finish()?;
                state.head(status)?;
                Ok(if status.is_informational() {
                    ResponseBlock::Informational
                } else {
                    ResponseBlock::Final {
                        status,
                        headers,
                        length,
                    }
                })
            }
            Collect::Trailers(fields) => {
                state.trailers()?;
                match self.error {
                    Some(error) => Err(error),
                    None => Ok(ResponseBlock::Trailers(fields)),
                }
            }
        }
    }
}

#[derive(Default)]
struct ResponseHead {
    status: Option<StatusCode>,
    regular: bool,
    headers: HeaderMap,
    length: Option<u64>,
}

fn response_field(
    name: &[u8],
    value: &[u8],
    typed: Option<(HeaderName, HeaderValue)>,
) -> Result<(HeaderName, HeaderValue), &'static str> {
    if name.iter().any(u8::is_ascii_uppercase)
        || value.iter().any(|byte| matches!(byte, 0 | b'\r' | b'\n'))
        || matches!(value.first(), Some(b' ' | b'\t'))
        || matches!(value.last(), Some(b' ' | b'\t'))
    {
        return Err("response contains malformed fields");
    }
    typed.ok_or("response contains malformed fields")
}

impl ResponseHead {
    fn pseudo(&mut self, name: &[u8], value: &[u8]) -> Result<(), &'static str> {
        if self.regular || name != b":status" || self.status.is_some() {
            return Err("response contains malformed pseudo-headers");
        }
        let Ok(code) = StatusCode::from_bytes(value) else {
            return Err("response contains malformed :status pseudo-header");
        };
        self.status = Some(code);
        Ok(())
    }

    fn field(
        &mut self,
        field: Result<(HeaderName, HeaderValue), &'static str>,
    ) -> Result<(), &'static str> {
        self.regular = true;
        let (name, value) = field?;
        if name == http::header::CONTENT_LENGTH {
            if self.length.is_some() {
                return Err("response has multiple content-length fields");
            }
            self.length = Some(
                value
                    .to_str()
                    .ok()
                    .and_then(crate::util::content_length)
                    .ok_or("response has invalid content-length")?,
            );
        }
        self.headers.append(name, value);
        Ok(())
    }

    fn finish(self) -> Result<(StatusCode, HeaderMap, Option<u64>), &'static str> {
        self.status
            .map(|status| (status, self.headers, self.length))
            .ok_or("response missing :status pseudo-header")
    }
}
