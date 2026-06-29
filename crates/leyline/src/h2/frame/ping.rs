//! PING frame (RFC 9113 Section 6.7).

use bytes::{BufMut, Bytes};

use super::{FrameHeader, FrameType};
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;

/// Flags for PING frames.
pub mod flags {
    pub const ACK: u8 = 0x1;
}

/// PING frame — connection liveness check.
#[derive(Debug)]
pub struct PingFrame {
    /// Whether this is an ACK (response to a PING).
    pub ack: bool,
    /// 8 bytes of opaque data.
    pub payload: [u8; 8],
}

impl PingFrame {
    /// Parse from header + payload.
    pub fn parse(header: FrameHeader, payload: Bytes) -> Result<Self, H2Error> {
        if header.stream_id != 0 {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "PING on non-zero stream".into(),
            });
        }
        if payload.len() != 8 {
            return Err(H2Error::Connection {
                code: ErrorCode::FrameSizeError,
                reason: format!("PING must be 8 bytes, got {}", payload.len()),
            });
        }

        let ack = header.flags & flags::ACK != 0;
        let mut data = [0u8; 8];
        data.copy_from_slice(&payload[..8]);

        Ok(Self { ack, payload: data })
    }

    /// Encode to bytes.
    pub fn encode(&self, buf: &mut impl BufMut) {
        let header = FrameHeader {
            length: 8,
            frame_type: FrameType::Ping as u8,
            flags: if self.ack { flags::ACK } else { 0 },
            stream_id: 0,
        };
        header.encode(buf);
        buf.put_slice(&self.payload);
    }
}
