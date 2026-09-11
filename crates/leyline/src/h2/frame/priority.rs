use bytes::Bytes;

use super::headers::StreamDependency;
use super::{FrameHeader, be_u32};
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;

#[derive(Debug)]
pub struct PriorityFrame {
    pub stream_id: u32,
    pub dependency: StreamDependency,
}

impl PriorityFrame {
    pub fn parse(header: FrameHeader, payload: Bytes) -> Result<Self, H2Error> {
        if header.stream_id == 0 {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "PRIORITY on stream 0".into(),
            });
        }
        if payload.len() != 5 {
            return Err(H2Error::Connection {
                code: ErrorCode::FrameSizeError,
                reason: format!("PRIORITY must be 5 bytes, got {}", payload.len()),
            });
        }

        let dep_raw = be_u32(&payload[..4]);

        Ok(Self {
            stream_id: header.stream_id,
            dependency: StreamDependency {
                exclusive: dep_raw & 0x8000_0000 != 0,
                dependency_id: dep_raw & 0x7FFF_FFFF,
                weight: payload[4],
            },
        })
    }
}
