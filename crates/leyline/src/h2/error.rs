use crate::error::BodyLimit;
use crate::error::{Cause, Error, Kind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
#[non_exhaustive]
pub enum ErrorCode {
    NoError = 0x0,
    ProtocolError = 0x1,
    InternalError = 0x2,
    FlowControlError = 0x3,
    SettingsTimeout = 0x4,
    StreamClosed = 0x5,
    FrameSizeError = 0x6,
    RefusedStream = 0x7,
    Cancel = 0x8,
    CompressionError = 0x9,
    ConnectError = 0xa,
    EnhanceYourCalm = 0xb,
    InadequateSecurity = 0xc,
    Http11Required = 0xd,
}

impl ErrorCode {
    pub fn from_u32(val: u32) -> Self {
        match val {
            0x0 => Self::NoError,
            0x1 => Self::ProtocolError,
            0x2 => Self::InternalError,
            0x3 => Self::FlowControlError,
            0x4 => Self::SettingsTimeout,
            0x5 => Self::StreamClosed,
            0x6 => Self::FrameSizeError,
            0x7 => Self::RefusedStream,
            0x8 => Self::Cancel,
            0x9 => Self::CompressionError,
            0xa => Self::ConnectError,
            0xb => Self::EnhanceYourCalm,
            0xc => Self::InadequateSecurity,
            0xd => Self::Http11Required,
            _ => Self::InternalError,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum H2Error {
    #[error("connection error: {code:?} - {reason}")]
    Connection { code: ErrorCode, reason: String },

    #[error("stream {stream_id} error: {code:?}")]
    Stream { stream_id: u32, code: ErrorCode },

    #[error("request not sent: the connection is closing")]
    NotSent,

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("hpack: {0}")]
    Hpack(String),

    #[error("frame size {size} exceeds max {max}")]
    FrameTooLarge { size: u32, max: u32 },

    #[error("request body stream failed")]
    RequestBody(#[source] std::io::Error),

    #[error("{0}")]
    BodyLimit(BodyLimit),

    #[error("peer did not advertise SETTINGS_ENABLE_CONNECT_PROTOCOL=1")]
    NoExtendedConnect,
}

fn copy_io(io: &std::io::Error) -> std::io::Error {
    std::io::Error::new(io.kind(), io.to_string())
}

impl H2Error {
    pub(crate) fn duplicate(&self) -> Self {
        match self {
            Self::Connection { code, reason } => Self::Connection {
                code: *code,
                reason: reason.clone(),
            },
            Self::Stream { stream_id, code } => Self::Stream {
                stream_id: *stream_id,
                code: *code,
            },
            Self::NotSent => Self::NotSent,
            Self::Io(io) => Self::Io(copy_io(io)),
            Self::Hpack(message) => Self::Hpack(message.clone()),
            Self::FrameTooLarge { size, max } => Self::FrameTooLarge {
                size: *size,
                max: *max,
            },
            Self::RequestBody(io) => Self::RequestBody(copy_io(io)),
            Self::BodyLimit(limit) => Self::BodyLimit(*limit),
            Self::NoExtendedConnect => Self::NoExtendedConnect,
        }
    }

    /// Returns true when the error ends the whole connection, not one stream.
    pub(crate) fn connection_failed(&self) -> bool {
        matches!(
            self,
            Self::Connection { .. }
                | Self::NotSent
                | Self::Io(_)
                | Self::Hpack(_)
                | Self::FrameTooLarge { .. }
        )
    }

    /// Returns true when the peer never processed the request, so it is safe to send again.
    pub(crate) fn refused(&self) -> bool {
        matches!(
            self,
            Self::NotSent
                | Self::Stream {
                    code: ErrorCode::RefusedStream,
                    ..
                }
        )
    }

    /// Returns the RFC 9113 error code the error carries.
    pub(crate) fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::Connection { code, .. } | Self::Stream { code, .. } => Some(*code),
            Self::NotSent => Some(ErrorCode::RefusedStream),
            _ => None,
        }
    }
}

impl From<H2Error> for Error {
    fn from(e: H2Error) -> Self {
        match e {
            H2Error::BodyLimit(limit) => limit.error(),
            H2Error::RequestBody(io) => Error::from_request_body(io),
            other => Error::new(Kind::Http2).with_cause(other),
        }
    }
}

impl Cause for H2Error {
    fn is_closed(&self) -> bool {
        matches!(
            self,
            Self::Io(_)
                | Self::Connection {
                    code: ErrorCode::NoError,
                    ..
                }
        )
    }

    fn refused(&self) -> bool {
        H2Error::refused(self)
    }

    fn h2_code(&self) -> Option<u32> {
        self.code().map(|code| code as u32)
    }

    fn duplicate(&self) -> Option<Box<dyn Cause>> {
        Some(Box::new(H2Error::duplicate(self)))
    }
}

#[cfg(test)]
impl Error {
    pub(crate) fn h2(&self) -> Option<&H2Error> {
        self.source_as()
    }
}
