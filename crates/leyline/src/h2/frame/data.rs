//! DATA frame (RFC 9113 Section 6.1).

use bytes::{BufMut, Bytes};

use super::{FrameHeader, FrameType};
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;

/// Flags for DATA frames.
pub mod flags {
    /// END_STREAM (0x1) — last frame for this stream.
    pub const END_STREAM: u8 = 0x1;
    /// PADDED (0x8) — frame is padded.
    pub const PADDED: u8 = 0x8;
}

/// DATA frame — carries request/response body data.
#[derive(Debug)]
pub struct DataFrame {
    /// Stream identifier (must be non-zero).
    pub stream_id: u32,
    /// Whether this is the last frame for the stream.
    pub end_stream: bool,
    /// Payload data (padding removed).
    pub data: Bytes,
    /// Full frame payload length, including the pad-length octet and padding when the `PADDED` flag is set.
    pub wire_len: u64,
}

impl DataFrame {
    /// Parse from header + payload.
    pub fn parse(header: FrameHeader, payload: Bytes) -> Result<Self, H2Error> {
        if header.stream_id == 0 {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "DATA frame on stream 0".into(),
            });
        }

        let end_stream = header.flags & flags::END_STREAM != 0;
        let padded = header.flags & flags::PADDED != 0;

        let data = if padded && !payload.is_empty() {
            let pad_len = payload[0] as usize;
            if pad_len >= payload.len() {
                return Err(H2Error::Connection {
                    code: ErrorCode::ProtocolError,
                    reason: "DATA padding exceeds payload".into(),
                });
            }
            payload.slice(1..payload.len() - pad_len)
        } else {
            payload
        };

        Ok(Self {
            stream_id: header.stream_id,
            end_stream,
            data,
            wire_len: header.length as u64,
        })
    }

    /// Encode to bytes (header + payload).
    pub fn encode(&self, buf: &mut impl BufMut) {
        let mut flag = 0u8;
        if self.end_stream {
            flag |= flags::END_STREAM;
        }
        let header = FrameHeader {
            length: self.data.len() as u32,
            frame_type: FrameType::Data as u8,
            flags: flag,
            stream_id: self.stream_id,
        };
        header.encode(buf);
        buf.put_slice(&self.data);
    }
}

#[cfg(test)]
mod tests;
