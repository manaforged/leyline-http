//! GOAWAY frame (RFC 9113 Section 6.8).

use bytes::{BufMut, Bytes};

use super::FrameHeader;
use crate::error::ErrorCode;
use crate::H2Error;

/// GOAWAY frame — signals connection shutdown.
#[derive(Debug)]
pub struct GoAwayFrame {
    /// Highest stream ID that was processed.
    pub last_stream_id: u32,
    /// Error code.
    pub error_code: ErrorCode,
    /// Optional debug data.
    pub debug_data: Bytes,
}

impl GoAwayFrame {
    /// Parse from header + payload.
    pub fn parse(header: FrameHeader, payload: Bytes) -> Result<Self, H2Error> {
        if header.stream_id != 0 {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "GOAWAY on non-zero stream".into(),
            });
        }
        if payload.len() < 8 {
            return Err(H2Error::Connection {
                code: ErrorCode::FrameSizeError,
                reason: format!("GOAWAY must be at least 8 bytes, got {}", payload.len()),
            });
        }

        let last_stream_id = ((payload[0] as u32) << 24)
            | ((payload[1] as u32) << 16)
            | ((payload[2] as u32) << 8)
            | (payload[3] as u32);
        let last_stream_id = last_stream_id & 0x7FFF_FFFF;

        let error_code = ((payload[4] as u32) << 24)
            | ((payload[5] as u32) << 16)
            | ((payload[6] as u32) << 8)
            | (payload[7] as u32);

        let debug_data = if payload.len() > 8 {
            payload.slice(8..)
        } else {
            Bytes::new()
        };

        Ok(Self {
            last_stream_id,
            error_code: ErrorCode::from_u32(error_code),
            debug_data,
        })
    }

    /// Encode to bytes.
    pub fn encode(&self, buf: &mut impl BufMut) {
        let length = 8 + self.debug_data.len() as u32;
        let header = FrameHeader {
            length,
            frame_type: 0x7,
            flags: 0,
            stream_id: 0,
        };
        header.encode(buf);
        buf.put_u32(self.last_stream_id & 0x7FFF_FFFF);
        buf.put_u32(self.error_code as u32);
        if !self.debug_data.is_empty() {
            buf.put_slice(&self.debug_data);
        }
    }
}
