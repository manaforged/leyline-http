use crate::core::session::decompress::BodyLimit;

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

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("hpack: {0}")]
    Hpack(String),

    #[error("frame size {size} exceeds max {max}")]
    FrameTooLarge { size: u32, max: u32 },
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
            Self::Io(io) => Self::Io(std::io::Error::new(io.kind(), io.to_string())),
            Self::Hpack(message) => Self::Hpack(message.clone()),
            Self::FrameTooLarge { size, max } => Self::FrameTooLarge {
                size: *size,
                max: *max,
            },
        }
    }

    pub(crate) fn body_limit(&self) -> Option<BodyLimit> {
        match self {
            Self::Io(io) => BodyLimit::of_io(io),
            _ => None,
        }
    }
}
