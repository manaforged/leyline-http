//! PUSH_PROMISE frame (RFC 9113 Section 6.6).
//!
//! Clients receive these from servers. We parse them so we can RST_STREAM
//! the promised stream (Chrome's behavior).

use bytes::Bytes;

use super::{FrameHeader, be_u32};
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;

/// PUSH_PROMISE frame — server push.
#[derive(Debug)]
pub struct PushPromiseFrame {
    /// Stream the promise is associated with.
    pub stream_id: u32,
    /// `END_HEADERS` flag — no CONTINUATION frames follow.
    pub end_headers: bool,
    /// The promised stream ID.
    pub promised_stream_id: u32,
    /// HPACK-encoded header block fragment.
    pub fragment: Bytes,
}

impl PushPromiseFrame {
    /// Parse from header + payload.
    pub fn parse(header: FrameHeader, payload: Bytes) -> Result<Self, H2Error> {
        if header.stream_id == 0 {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "PUSH_PROMISE on stream 0".into(),
            });
        }

        let end_headers = header.flags & 0x4 != 0;
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
            stream_id: header.stream_id,
            end_headers,
            promised_stream_id,
            fragment,
        })
    }
}
