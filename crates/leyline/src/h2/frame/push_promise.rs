use bytes::Bytes;

use super::{FrameHeader, be_u32};
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;

#[derive(Debug)]
pub struct PushPromiseFrame {
    pub promised_stream_id: u32,
    pub fragment: Bytes,
}

impl PushPromiseFrame {
    pub fn parse(header: FrameHeader, payload: Bytes) -> Result<Self, H2Error> {
        if header.stream_id == 0 {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "PUSH_PROMISE on stream 0".into(),
            });
        }

        let padded = header.flags & 0x8 != 0;

        let mut offset = 0;
        let mut end = payload.len();

        if padded {
            if payload.is_empty() {
                return Err(H2Error::Connection {
                    code: ErrorCode::ProtocolError,
                    reason: "PUSH_PROMISE PADDED but no payload".into(),
                });
            }
            let pad_len = payload[0] as usize;
            offset = 1;
            if pad_len >= end - offset {
                return Err(H2Error::Connection {
                    code: ErrorCode::ProtocolError,
                    reason: "PUSH_PROMISE padding exceeds payload".into(),
                });
            }
            end -= pad_len;
        }

        if offset + 4 > end {
            return Err(H2Error::Connection {
                code: ErrorCode::FrameSizeError,
                reason: "PUSH_PROMISE too short for promised stream ID".into(),
            });
        }

        let promised = be_u32(&payload[offset..offset + 4]);
        let promised_stream_id = promised & 0x7FFF_FFFF;
        offset += 4;

        let fragment = payload.slice(offset..end);

        Ok(Self {
            promised_stream_id,
            fragment,
        })
    }
}
