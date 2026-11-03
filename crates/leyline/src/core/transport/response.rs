use http::StatusCode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResponseState {
    Initial(usize),
    Final,
    Trailers,
}

impl ResponseState {
    pub(crate) fn head(&mut self, status: StatusCode) -> Result<(), &'static str> {
        let Self::Initial(informational) = self else {
            return Err("response head arrived after a final response head");
        };
        if status.is_informational() {
            if *informational >= crate::util::MAX_INFORMATIONAL {
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
