//! PRIORITY frame (RFC 9113 Section 6.3).
//!
//! Deprecated in RFC 9113 but must be accepted without error.

use bytes::{BufMut, Bytes};

use super::headers::StreamDependency;
use super::FrameHeader;
use crate::error::ErrorCode;
use crate::H2Error;

/// PRIORITY frame — stream dependency (deprecated, parse only).
#[derive(Debug)]
pub struct PriorityFrame {
    /// Stream whose priority is being advised.
    pub stream_id: u32,
    /// Dependency declaration carried by the frame.
    pub dependency: StreamDependency,
}

impl PriorityFrame {
    /// Parse from header + payload.
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

        let dep_raw = ((payload[0] as u32) << 24)
            | ((payload[1] as u32) << 16)
            | ((payload[2] as u32) << 8)
            | (payload[3] as u32);

        Ok(Self {
            stream_id: header.stream_id,
            dependency: StreamDependency {
                exclusive: dep_raw & 0x8000_0000 != 0,
                dependency_id: dep_raw & 0x7FFF_FFFF,
                weight: payload[4],
            },
        })
    }

    /// Encode to bytes.
    pub fn encode(&self, buf: &mut impl BufMut) {
        let header = FrameHeader {
            length: 5,
            frame_type: 0x2,
            flags: 0,
            stream_id: self.stream_id,
        };
        header.encode(buf);
        let mut dep = self.dependency.dependency_id & 0x7FFF_FFFF;
        if self.dependency.exclusive {
            dep |= 0x8000_0000;
        }
        buf.put_u32(dep);
        buf.put_u8(self.dependency.weight);
    }
}
