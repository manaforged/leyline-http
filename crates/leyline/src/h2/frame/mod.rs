#![forbid(unsafe_code)]
mod data;
mod goaway;
mod headers;
mod ping;
mod priority;
mod push_promise;
mod rst_stream;
mod settings;
mod window_update;

pub use data::DataFrame;
pub use goaway::GoAwayFrame;
pub use headers::{HeadersFrame, StreamDependency};
pub use ping::PingFrame;
pub use push_promise::PushPromiseFrame;
pub use rst_stream::RstStreamFrame;
pub use settings::SettingsFrame;
pub use window_update::WindowUpdateFrame;

use bytes::{BufMut, Bytes};

use crate::h2::H2Error;
use crate::h2::error::ErrorCode;

/// The largest value a 31-bit HTTP/2 field holds: a stream ID, window, or window increment.
pub const U31_MAX: u32 = 0x7FFF_FFFF;
/// The RFC 9113 initial SETTINGS_MAX_FRAME_SIZE.
pub(crate) const DEFAULT_MAX_FRAME_SIZE: u32 = 16_384;
/// The RFC 9113 initial SETTINGS_HEADER_TABLE_SIZE.
pub(crate) const DEFAULT_HEADER_TABLE_SIZE: u32 = 4096;
/// The largest SETTINGS_MAX_FRAME_SIZE a peer may advertise.
pub(crate) const MAX_FRAME_SIZE_LIMIT: u32 = (1 << 24) - 1;
/// The length of the stream dependency and weight a PRIORITY flag adds.
pub(crate) const PRIORITY_LEN: usize = 5;

/// HTTP/2 frame flag bits.
pub mod flags {
    /// END_STREAM on DATA and HEADERS.
    pub const END_STREAM: u8 = 0x1;
    /// ACK on SETTINGS and PING.
    pub const ACK: u8 = 0x1;
    /// END_HEADERS on HEADERS, PUSH_PROMISE and CONTINUATION.
    pub const END_HEADERS: u8 = 0x4;
    /// PADDED on DATA, HEADERS and PUSH_PROMISE.
    pub const PADDED: u8 = 0x8;
    /// PRIORITY on HEADERS.
    pub const PRIORITY: u8 = 0x20;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    Stream,
    Connection,
}

pub(super) fn check_scope(header: &FrameHeader, scope: Scope, frame: &str) -> Result<(), H2Error> {
    let on_connection = header.stream_id == 0;
    if on_connection == (scope == Scope::Connection) {
        return Ok(());
    }
    let place = if on_connection {
        "stream 0"
    } else {
        "non-zero stream"
    };
    Err(H2Error::Connection {
        code: ErrorCode::ProtocolError,
        reason: format!("{frame} on {place}"),
    })
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

pub(crate) struct ContinuationFrame<'a> {
    pub(crate) stream_id: u32,
    pub(crate) end_headers: bool,
    pub(crate) fragment: &'a [u8],
}

impl ContinuationFrame<'_> {
    pub(crate) fn encode(&self, buf: &mut impl BufMut) {
        FrameHeader {
            length: self.fragment.len() as u32,
            frame_type: FrameType::Continuation as u8,
            flags: if self.end_headers {
                flags::END_HEADERS
            } else {
                0
            },
            stream_id: self.stream_id,
        }
        .encode(buf);
        buf.put_slice(self.fragment);
    }
}

#[inline]
pub(crate) fn be_u16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

#[inline]
pub(crate) fn be_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    Data = 0x0,
    Headers = 0x1,
    Priority = 0x2,
    RstStream = 0x3,
    Settings = 0x4,
    PushPromise = 0x5,
    Ping = 0x6,
    GoAway = 0x7,
    WindowUpdate = 0x8,
    Continuation = 0x9,
}

impl FrameType {
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0x0 => Some(Self::Data),
            0x1 => Some(Self::Headers),
            0x2 => Some(Self::Priority),
            0x3 => Some(Self::RstStream),
            0x4 => Some(Self::Settings),
            0x5 => Some(Self::PushPromise),
            0x6 => Some(Self::Ping),
            0x7 => Some(Self::GoAway),
            0x8 => Some(Self::WindowUpdate),
            0x9 => Some(Self::Continuation),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FrameHeader {
    pub length: u32,
    pub frame_type: u8,
    pub flags: u8,
    pub stream_id: u32,
}

pub const FRAME_HEADER_LEN: usize = 9;

impl FrameHeader {
    pub fn parse(buf: &[u8; FRAME_HEADER_LEN]) -> Self {
        let length = ((buf[0] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[2] as u32);
        let frame_type = buf[3];
        let flags = buf[4];
        let stream_id = be_u32(&buf[5..9]) & U31_MAX;

        Self {
            length,
            frame_type,
            flags,
            stream_id,
        }
    }

    pub fn encode(&self, buf: &mut impl BufMut) {
        buf.put_u8((self.length >> 16) as u8);
        buf.put_u8((self.length >> 8) as u8);
        buf.put_u8(self.length as u8);
        buf.put_u8(self.frame_type);
        buf.put_u8(self.flags);
        buf.put_u32(self.stream_id & U31_MAX);
    }
}

#[derive(Debug)]
pub enum Frame {
    Data(DataFrame),
    Headers(HeadersFrame),
    Priority,
    RstStream(RstStreamFrame),
    Settings(SettingsFrame),
    PushPromise(PushPromiseFrame),
    Ping(PingFrame),
    GoAway(GoAwayFrame),
    WindowUpdate(WindowUpdateFrame),
    Continuation {
        stream_id: u32,
        end_headers: bool,
        fragment: Bytes,
    },
    Unknown,
}

impl Frame {
    pub fn parse(header: FrameHeader, payload: Bytes) -> Result<Self, H2Error> {
        let Some(kind) = FrameType::from_u8(header.frame_type) else {
            return Ok(Frame::Unknown);
        };
        match kind {
            FrameType::Data => DataFrame::parse(header, payload).map(Frame::Data),
            FrameType::Headers => HeadersFrame::parse(header, payload).map(Frame::Headers),
            FrameType::Priority => priority::validate(&header, &payload).map(|()| Frame::Priority),
            FrameType::RstStream => RstStreamFrame::parse(header, payload).map(Frame::RstStream),
            FrameType::Settings => SettingsFrame::parse(header, payload).map(Frame::Settings),
            FrameType::PushPromise => {
                PushPromiseFrame::parse(header, payload).map(Frame::PushPromise)
            }
            FrameType::Ping => PingFrame::parse(header, payload).map(Frame::Ping),
            FrameType::GoAway => GoAwayFrame::parse(header, payload).map(Frame::GoAway),
            FrameType::WindowUpdate => {
                WindowUpdateFrame::parse(header, payload).map(Frame::WindowUpdate)
            }
            FrameType::Continuation => Ok(Frame::Continuation {
                stream_id: header.stream_id,
                end_headers: header.flags & flags::END_HEADERS != 0,
                fragment: payload,
            }),
        }
    }
}

#[cfg(kani)]
mod proofs;
#[cfg(test)]
#[path = "rfc_tests.rs"]
mod rfc_tests;
#[cfg(test)]
mod tests;
