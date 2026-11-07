use bytes::{BufMut, Bytes};

use super::{FrameHeader, FrameType, be_u32};
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;

pub mod flags {
    pub const END_STREAM: u8 = 0x1;
    pub const END_HEADERS: u8 = 0x4;
    pub const PADDED: u8 = 0x8;
    pub const PRIORITY: u8 = 0x20;
}

#[derive(Debug, Clone, Copy)]
pub struct StreamDependency {
    pub exclusive: bool,
    pub dependency_id: u32,
    pub weight: u8,
}

#[derive(Debug)]
pub struct HeadersFrame<F = Bytes> {
    pub stream_id: u32,
    pub end_stream: bool,
    pub end_headers: bool,
    pub priority: Option<StreamDependency>,
    pub fragment: F,
}

impl HeadersFrame {
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

        let mut offset = usize::from(padded);
        let end = padded_end(&payload, padded, if has_priority { 5 } else { 0 })?;

        let priority = if has_priority {
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
}

/// Returns where the field block ends after RFC 9113 padding, checking frame size before padding.
pub(super) fn padded_end(payload: &[u8], padded: bool, fixed: usize) -> Result<usize, H2Error> {
    let mandatory = usize::from(padded) + fixed;
    if payload.len() < mandatory {
        return Err(H2Error::Connection {
            code: ErrorCode::FrameSizeError,
            reason: format!(
                "frame of {} bytes is shorter than its {mandatory} fixed bytes",
                payload.len()
            ),
        });
    }
    let pad_len = if padded { usize::from(payload[0]) } else { 0 };
    if pad_len > payload.len() - mandatory {
        return Err(H2Error::Connection {
            code: ErrorCode::ProtocolError,
            reason: "padding exceeds the field block".into(),
        });
    }
    Ok(payload.len() - pad_len)
}

impl<F: AsRef<[u8]>> HeadersFrame<F> {
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
        let fragment = self.fragment.as_ref();
        let length = priority_len + fragment.len() as u32;

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

        buf.put_slice(fragment);
    }
}

#[cfg(test)]
mod tests;
