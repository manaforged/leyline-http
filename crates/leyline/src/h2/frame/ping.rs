use bytes::{BufMut, Bytes};

use super::{FrameHeader, FrameType, Scope, check_scope, flags};
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;

#[derive(Debug)]
pub struct PingFrame {
    pub ack: bool,
    pub payload: [u8; 8],
}

impl PingFrame {
    pub fn parse(header: FrameHeader, payload: Bytes) -> Result<Self, H2Error> {
        check_scope(&header, Scope::Connection, "PING")?;
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
