use bytes::{BufMut, Bytes};

use super::{
    FrameHeader, FrameType, PRIORITY_LEN, Scope, U31_MAX, be_u32, check_scope, flags, padded_end,
};
use crate::h2::H2Error;

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
        check_scope(&header, Scope::Stream, "HEADERS")?;

        let end_stream = header.flags & flags::END_STREAM != 0;
        let end_headers = header.flags & flags::END_HEADERS != 0;
        let padded = header.flags & flags::PADDED != 0;
        let has_priority = header.flags & flags::PRIORITY != 0;

        let mut offset = usize::from(padded);
        let end = padded_end(
            &payload,
            padded,
            if has_priority { PRIORITY_LEN } else { 0 },
        )?;

        let priority = if has_priority {
            let dep_raw = be_u32(&payload[offset..offset + 4]);
            let exclusive = dep_raw & 0x8000_0000 != 0;
            let dependency_id = dep_raw & U31_MAX;
            let weight = payload[offset + 4];
            offset += PRIORITY_LEN;
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

        let priority_len = if self.priority.is_some() {
            PRIORITY_LEN as u32
        } else {
            0
        };
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
            let mut dep_val = dep.dependency_id & U31_MAX;
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
