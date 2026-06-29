//! SETTINGS frame (RFC 9113 Section 6.5).

use bytes::{BufMut, Bytes};

use super::{FrameHeader, FrameType, be_u16, be_u32};
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;

/// Flags for SETTINGS frames.
pub mod flags {
    /// ACK (0x1) — acknowledges receipt of SETTINGS.
    pub const ACK: u8 = 0x1;
}

/// SETTINGS frame — connection configuration parameters.
#[derive(Debug)]
pub struct SettingsFrame {
    /// Whether this is an ACK frame.
    pub ack: bool,
    /// Settings parameters in wire order (order matters for fingerprinting).
    pub params: Vec<(u16, u32)>,
}

impl SettingsFrame {
    /// Parse from header + payload.
    pub fn parse(header: FrameHeader, payload: Bytes) -> Result<Self, H2Error> {
        if header.stream_id != 0 {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "SETTINGS frame on non-zero stream".into(),
            });
        }

        let ack = header.flags & flags::ACK != 0;

        if ack && !payload.is_empty() {
            return Err(H2Error::Connection {
                code: ErrorCode::FrameSizeError,
                reason: "SETTINGS ACK with non-empty payload".into(),
            });
        }

        if payload.len() % 6 != 0 {
            return Err(H2Error::Connection {
                code: ErrorCode::FrameSizeError,
                reason: "SETTINGS payload not multiple of 6".into(),
            });
        }

        let mut params = Vec::with_capacity(payload.len() / 6);
        let mut i = 0;
        while i + 6 <= payload.len() {
            let id = be_u16(&payload[i..i + 2]);
            let val = be_u32(&payload[i + 2..i + 6]);
            params.push((id, val));
            i += 6;
        }

        Ok(Self { ack, params })
    }

    /// Create a SETTINGS ACK frame.
    pub fn ack() -> Self {
        Self {
            ack: true,
            params: Vec::new(),
        }
    }

    /// Encode to bytes (header + payload).
    pub fn encode(&self, buf: &mut impl BufMut) {
        let payload_len = if self.ack { 0 } else { self.params.len() * 6 };
        let header = FrameHeader {
            length: payload_len as u32,
            frame_type: FrameType::Settings as u8,
            flags: if self.ack { flags::ACK } else { 0 },
            stream_id: 0,
        };
        header.encode(buf);

        for &(id, val) in &self.params {
            buf.put_u16(id);
            buf.put_u32(val);
        }
    }
}
