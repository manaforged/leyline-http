use bytes::Bytes;

use super::{FrameHeader, Scope, U31_MAX, be_u32, check_scope, flags, padded_end};
use crate::h2::H2Error;

#[derive(Debug)]
pub struct PushPromiseFrame {
    pub stream_id: u32,
    pub end_headers: bool,
    pub promised_stream_id: u32,
    pub fragment: Bytes,
}

impl PushPromiseFrame {
    pub fn parse(header: FrameHeader, payload: Bytes) -> Result<Self, H2Error> {
        check_scope(&header, Scope::Stream, "PUSH_PROMISE")?;

        let padded = header.flags & flags::PADDED != 0;
        let end_headers = header.flags & flags::END_HEADERS != 0;

        let mut offset = usize::from(padded);
        let end = padded_end(&payload, padded, 4)?;

        let promised_stream_id = be_u32(&payload[offset..offset + 4]) & U31_MAX;
        offset += 4;

        let fragment = payload.slice(offset..end);

        Ok(Self {
            stream_id: header.stream_id,
            end_headers,
            promised_stream_id,
            fragment,
        })
    }
}
