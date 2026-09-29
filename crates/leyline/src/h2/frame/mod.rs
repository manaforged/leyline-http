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

#[cfg(test)]
use bytes::BytesMut;
use bytes::{BufMut, Bytes};

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
        let stream_id = be_u32(&buf[5..9]);
        let stream_id = stream_id & 0x7FFF_FFFF;

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
        buf.put_u32(self.stream_id & 0x7FFF_FFFF);
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
    pub fn parse(header: FrameHeader, payload: Bytes) -> Result<Self, crate::h2::H2Error> {
        match FrameType::from_u8(header.frame_type) {
            Some(FrameType::Data) => Ok(Frame::Data(DataFrame::parse(header, payload)?)),
            Some(FrameType::Headers) => Ok(Frame::Headers(HeadersFrame::parse(header, payload)?)),
            Some(FrameType::Priority) => {
                priority::validate(&header, &payload)?;
                Ok(Frame::Priority)
            }
            Some(FrameType::RstStream) => {
                Ok(Frame::RstStream(RstStreamFrame::parse(header, payload)?))
            }
            Some(FrameType::Settings) => {
                Ok(Frame::Settings(SettingsFrame::parse(header, payload)?))
            }
            Some(FrameType::PushPromise) => Ok(Frame::PushPromise(PushPromiseFrame::parse(
                header, payload,
            )?)),
            Some(FrameType::Ping) => Ok(Frame::Ping(PingFrame::parse(header, payload)?)),
            Some(FrameType::GoAway) => Ok(Frame::GoAway(GoAwayFrame::parse(header, payload)?)),
            Some(FrameType::WindowUpdate) => Ok(Frame::WindowUpdate(WindowUpdateFrame::parse(
                header, payload,
            )?)),
            Some(FrameType::Continuation) => {
                let end_headers = header.flags & 0x4 != 0;
                Ok(Frame::Continuation {
                    stream_id: header.stream_id,
                    end_headers,
                    fragment: payload,
                })
            }
            None => Ok(Frame::Unknown),
        }
    }
}

#[cfg(test)]
mod tests;
