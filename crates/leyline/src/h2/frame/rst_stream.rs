//! RST_STREAM frame (RFC 9113 Section 6.4).

use bytes::{BufMut, Bytes};

use super::{be_u32, FrameHeader, FrameType};
use crate::h2::error::ErrorCode;
use crate::h2::H2Error;

/// RST_STREAM frame — abruptly terminates a stream.
#[derive(Debug)]
pub struct RstStreamFrame {
    /// Stream being terminated.
    pub stream_id: u32,
    /// Reason the stream is being reset.
    pub error_code: ErrorCode,
}

impl RstStreamFrame {
    /// Parse from header + payload.
    pub fn parse(header: FrameHeader, payload: Bytes) -> Result<Self, H2Error> {
        if header.stream_id == 0 {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "RST_STREAM on stream 0".into(),
            });
        }
        if payload.len() != 4 {
            return Err(H2Error::Connection {
                code: ErrorCode::FrameSizeError,
                reason: format!("RST_STREAM must be 4 bytes, got {}", payload.len()),
            });
        }

        let code = be_u32(&payload[..4]);

        Ok(Self {
            stream_id: header.stream_id,
            error_code: ErrorCode::from_u32(code),
        })
    }

    /// Encode to bytes.
    pub fn encode(&self, buf: &mut impl BufMut) {
        let header = FrameHeader {
            length: 4,
            frame_type: FrameType::RstStream as u8,
            flags: 0,
            stream_id: self.stream_id,
        };
        header.encode(buf);
        buf.put_u32(self.error_code as u32);
    }
}
