//! HEADERS frame (RFC 9113 Section 6.2).

use bytes::{BufMut, Bytes};

use super::{be_u32, FrameHeader, FrameType};
use crate::h2::error::ErrorCode;
use crate::h2::H2Error;

/// Flags for HEADERS frames.
pub mod flags {
    /// Marks the stream as half-closed after this frame.
    pub const END_STREAM: u8 = 0x1;
    /// Indicates that the header block ends with this frame.
    pub const END_HEADERS: u8 = 0x4;
    /// The payload starts with an octet count and trailing padding.
    pub const PADDED: u8 = 0x8;
    /// Priority fields are present before the header block.
    pub const PRIORITY: u8 = 0x20;
}

/// Stream dependency for PRIORITY-flagged HEADERS.
#[derive(Debug, Clone, Copy)]
pub struct StreamDependency {
    /// Whether this is an exclusive dependency.
    pub exclusive: bool,
    /// The stream this depends on.
    pub dependency_id: u32,
    /// Weight (1-256, wire value is 0-255).
    pub weight: u8,
}

/// HEADERS frame — carries header block fragment + optional priority.
#[derive(Debug)]
pub struct HeadersFrame {
    /// Stream this header block belongs to.
    pub stream_id: u32,
    /// `END_STREAM` flag — the sender won't send any more DATA on this stream.
    pub end_stream: bool,
    /// `END_HEADERS` flag — no CONTINUATION frames follow.
    pub end_headers: bool,
    /// Optional priority/dependency hint when the `PRIORITY` flag is set.
    pub priority: Option<StreamDependency>,
    /// Raw HPACK-encoded header block fragment.
    pub fragment: Bytes,
}

impl HeadersFrame {
    /// Parse from header + payload.
    pub fn parse(header: FrameHeader, payload: Bytes) -> Result<Self, H2Error> {
        if header.stream_id == 0 {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "HEADERS on stream 0".into(),
            });
        }

        let end_stream = header.flags & flags::END_STREAM != 0;
        let end_headers = header.flags & flags::END_HEADERS != 0;
        let padded = header.flags & flags::PADDED != 0;
        let has_priority = header.flags & flags::PRIORITY != 0;

        let mut offset = 0;
        let mut end = payload.len();

        // Handle padding.
        if padded {
            if payload.is_empty() {
                return Err(H2Error::Connection {
                    code: ErrorCode::ProtocolError,
                    reason: "HEADERS PADDED flag but no payload".into(),
                });
            }
            let pad_len = payload[0] as usize;
            offset = 1;
            if pad_len >= end - offset {
                return Err(H2Error::Connection {
                    code: ErrorCode::ProtocolError,
                    reason: "HEADERS padding exceeds payload".into(),
                });
            }
            end -= pad_len;
        }

        // Handle priority.
        let priority = if has_priority {
            if offset + 5 > end {
                return Err(H2Error::Connection {
                    code: ErrorCode::FrameSizeError,
                    reason: "HEADERS PRIORITY flag but insufficient payload".into(),
                });
            }
            let dep_raw = be_u32(&payload[offset..offset + 4]);
            let exclusive = dep_raw & 0x8000_0000 != 0;
            let dependency_id = dep_raw & 0x7FFF_FFFF;
            let weight = payload[offset + 4];
            offset += 5;
            Some(StreamDependency {
                exclusive,
                dependency_id,
                weight,
            })
        } else {
            None
        };

        let fragment = payload.slice(offset..end);

        Ok(Self {
            stream_id: header.stream_id,
            end_stream,
            end_headers,
            priority,
            fragment,
        })
    }

    /// Encode to bytes. The fragment must be pre-encoded via HPACK.
    pub fn encode(&self, buf: &mut impl BufMut) {
        let mut flags = 0u8;
        if self.end_stream {
            flags |= flags::END_STREAM;
        }
        if self.end_headers {
            flags |= flags::END_HEADERS;
        }
        if self.priority.is_some() {
            flags |= flags::PRIORITY;
        }

        let priority_len = if self.priority.is_some() { 5 } else { 0 };
        let length = priority_len + self.fragment.len() as u32;

        let header = FrameHeader {
            length,
            frame_type: FrameType::Headers as u8,
            flags,
            stream_id: self.stream_id,
        };
        header.encode(buf);

        if let Some(dep) = &self.priority {
            let mut dep_val = dep.dependency_id & 0x7FFF_FFFF;
            if dep.exclusive {
                dep_val |= 0x8000_0000;
            }
            buf.put_u32(dep_val);
            buf.put_u8(dep.weight);
        }

        buf.put_slice(&self.fragment);
    }
}
