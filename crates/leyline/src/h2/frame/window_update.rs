use bytes::{BufMut, Bytes};

use super::{FrameHeader, FrameType, be_u32};
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;

#[derive(Debug)]
pub struct WindowUpdateFrame {
    pub stream_id: u32,
    pub increment: u32,
}

impl WindowUpdateFrame {
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

        let increment = be_u32(&payload[..4]);
        let increment = increment & 0x7FFF_FFFF;
        if increment == 0 {
            if header.stream_id == 0 {
                return Err(H2Error::Connection {
                    code: ErrorCode::ProtocolError,
                    reason: "WINDOW_UPDATE increment of 0 on connection".into(),
                });
            } else {
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

    pub fn encode(&self, buf: &mut impl BufMut) {
        let header = FrameHeader {
            length: 4,
            frame_type: FrameType::WindowUpdate as u8,
            flags: 0,
            stream_id: self.stream_id,
        };
        header.encode(buf);
        buf.put_u32(self.increment & 0x7FFF_FFFF);
    }
}
