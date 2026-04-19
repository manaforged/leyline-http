//! WINDOW_UPDATE frame (RFC 9113 Section 6.9).

use bytes::{BufMut, Bytes};

use super::FrameHeader;
use crate::h2::error::ErrorCode;
use crate::h2::H2Error;

/// WINDOW_UPDATE frame — flow control increment.
#[derive(Debug)]
pub struct WindowUpdateFrame {
    /// Stream to credit (0 means connection-level window).
    pub stream_id: u32,
    /// Window size increment (1 to 2^31-1). Zero is a protocol error.
    pub increment: u32,
}

impl WindowUpdateFrame {
    /// Parse from header + payload.
    pub fn parse(header: FrameHeader, payload: Bytes) -> Result<Self, H2Error> {
        if payload.len() != 4 {
            return Err(H2Error::Connection {
                code: ErrorCode::FrameSizeError,
                reason: format!(
                    "WINDOW_UPDATE payload must be 4 bytes, got {}",
                    payload.len()
                ),
            });
        }

        let increment = ((payload[0] as u32) << 24)
            | ((payload[1] as u32) << 16)
            | ((payload[2] as u32) << 8)
            | (payload[3] as u32);
        let increment = increment & 0x7FFF_FFFF; // clear R bit

        if increment == 0 {
            if header.stream_id == 0 {
                // Connection-level zero increment → connection error.
                return Err(H2Error::Connection {
                    code: ErrorCode::ProtocolError,
                    reason: "WINDOW_UPDATE increment of 0 on connection".into(),
                });
            } else {
                // Stream-level zero increment → stream error.
                return Err(H2Error::Stream {
                    stream_id: header.stream_id,
                    code: ErrorCode::ProtocolError,
                });
            }
        }

        Ok(Self {
            stream_id: header.stream_id,
            increment,
        })
    }

    /// Encode to bytes.
    pub fn encode(&self, buf: &mut impl BufMut) {
        let header = FrameHeader {
            length: 4,
            frame_type: 0x8,
            flags: 0,
            stream_id: self.stream_id,
        };
        header.encode(buf);
        buf.put_u32(self.increment & 0x7FFF_FFFF);
    }
}
