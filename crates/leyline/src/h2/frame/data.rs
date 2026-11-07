use bytes::{BufMut, Bytes};

use super::{FrameHeader, FrameType, Scope, check_scope, flags, padded_end};
use crate::h2::H2Error;

#[derive(Debug)]
pub struct DataFrame {
    pub stream_id: u32,
    pub end_stream: bool,
    pub data: Bytes,
    pub wire_len: u64,
}

impl DataFrame {
    pub fn parse(header: FrameHeader, payload: Bytes) -> Result<Self, H2Error> {
        check_scope(&header, Scope::Stream, "DATA")?;

        let end_stream = header.flags & flags::END_STREAM != 0;
        let data = if header.flags & flags::PADDED != 0 {
            let end = padded_end(&payload, true, 0)?;
            payload.slice(1..end)
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
