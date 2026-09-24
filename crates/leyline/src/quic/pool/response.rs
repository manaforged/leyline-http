#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum H3ResponseState {
    Initial,
    Final,
    Trailers,
}

pub(super) type H3Headers = Vec<(String, String)>;

pub(super) enum H3HeaderBlock {
    Informational,
    Final { status: u16, headers: H3Headers },
    Trailers(H3Headers),
}

impl H3ResponseState {
    pub(super) fn headers(
        &mut self,
        list: &[(String, String)],
    ) -> Result<H3HeaderBlock, &'static str> {
        match self {
            Self::Initial => {
                let (status, headers) = parse_response_head(list)?;
                if (100..200).contains(&status) {
                    Ok(H3HeaderBlock::Informational)
                } else {
                    *self = Self::Final;
                    Ok(H3HeaderBlock::Final { status, headers })
                }
            }
            Self::Final => {
                if list.iter().any(|(name, _)| name.starts_with(':')) {
                    return Err("h3: trailers must not contain pseudo-headers");
                }
                *self = Self::Trailers;
                Ok(H3HeaderBlock::Trailers(list.to_vec()))
            }
            Self::Trailers => Err("h3: response contains headers after trailers"),
        }
    }

    pub(super) fn data(self) -> Result<(), &'static str> {
        match self {
            Self::Final => Ok(()),
            Self::Initial => Err("h3: response DATA arrived before a final response head"),
            Self::Trailers => Err("h3: response DATA arrived after trailers"),
        }
    }

    pub(super) fn finish(self) -> Result<(), &'static str> {
        match self {
            Self::Initial => Err("h3: response ended before a final response head"),
            Self::Final | Self::Trailers => Ok(()),
        }
    }
}

fn parse_response_head(list: &[(String, String)]) -> Result<(u16, H3Headers), &'static str> {
    let mut status = None;
    let mut headers = Vec::with_capacity(list.len());
    let mut regular = false;

    for (name, value) in list {
        if name.starts_with(':') {
            if regular || name != ":status" || status.is_some() {
                return Err("h3: response contains malformed pseudo-headers");
            }
            if value.len() != 3 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err("h3: response contains malformed :status pseudo-header");
            }
            let code = value
                .bytes()
                .fold(0, |status, byte| status * 10 + u16::from(byte - b'0'));
            if code == 101 {
                return Err("h3: status 101 is forbidden");
            }
            status = Some(code);
        } else {
            regular = true;
            headers.push((name.clone(), value.clone()));
        }
    }

    status
        .map(|status| (status, headers))
        .ok_or("h3: response missing :status pseudo-header")
}
