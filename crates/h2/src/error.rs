//! HTTP/2 error types.

/// HTTP/2 error codes (RFC 9113 Section 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ErrorCode {
    /// Graceful shutdown.
    NoError = 0x0,
    /// Protocol error detected.
    ProtocolError = 0x1,
    /// Internal error.
    InternalError = 0x2,
    /// Flow control limits exceeded.
    FlowControlError = 0x3,
    /// Settings not acknowledged in time.
    SettingsTimeout = 0x4,
    /// Stream is half-closed.
    StreamClosed = 0x5,
    /// Frame size incorrect.
    FrameSizeError = 0x6,
    /// Stream not processed, can retry.
    RefusedStream = 0x7,
    /// Stream cancelled.
    Cancel = 0x8,
    /// Compression state not updated.
    CompressionError = 0x9,
    /// TCP connection timed out.
    ConnectError = 0xa,
    /// Peer exceeded concurrent stream limit.
    EnhanceYourCalm = 0xb,
    /// Underlying transport security not adequate.
    InadequateSecurity = 0xc,
    /// Endpoint requires HTTP/1.1.
    Http11Required = 0xd,
}

impl ErrorCode {
    /// Parse from a u32 wire value.
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
            _ => Self::InternalError, // unknown error codes treated as internal
        }
    }
}

/// HTTP/2 errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum H2Error {
    /// Connection-level protocol error.
    #[error("connection error: {code:?} - {reason}")]
    Connection {
        /// The error code.
        code: ErrorCode,
        /// Human-readable reason.
        reason: String,
    },

    /// Stream-level error.
    #[error("stream {stream_id} error: {code:?}")]
    Stream {
        /// The stream that errored.
        stream_id: u32,
        /// The error code.
        code: ErrorCode,
    },

    /// IO error from the underlying transport.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// HPACK decompression error.
    #[error("hpack: {0}")]
    Hpack(String),

    /// Frame too large.
    #[error("frame size {size} exceeds max {max}")]
    FrameTooLarge {
        /// Actual size.
        size: u32,
        /// Maximum allowed.
        max: u32,
    },
}
