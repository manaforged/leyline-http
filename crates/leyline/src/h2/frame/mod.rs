//! HTTP/2 frame types and codec.
//!
//! All 10 frame types from RFC 9113 Section 6, plus the 9-byte frame header.

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
pub use priority::PriorityFrame;
pub use push_promise::PushPromiseFrame;
pub use rst_stream::RstStreamFrame;
pub use settings::SettingsFrame;
pub use window_update::WindowUpdateFrame;

#[cfg(test)]
use bytes::BytesMut;
use bytes::{BufMut, Bytes};

/// Frame type IDs (RFC 9113 Section 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    /// DATA frame — carries HTTP request/response body bytes.
    Data = 0x0,
    /// HEADERS frame — carries a HPACK-encoded header block fragment.
    Headers = 0x1,
    /// PRIORITY frame — conveys sender-advised stream priority.
    Priority = 0x2,
    /// RST_STREAM frame — abruptly terminates a stream.
    RstStream = 0x3,
    /// SETTINGS frame — conveys connection-level configuration.
    Settings = 0x4,
    /// PUSH_PROMISE frame — reserves a stream for server push.
    PushPromise = 0x5,
    /// PING frame — liveness check and round-trip time measurement.
    Ping = 0x6,
    /// GOAWAY frame — initiates graceful shutdown of a connection.
    GoAway = 0x7,
    /// WINDOW_UPDATE frame — extends flow-control credit.
    WindowUpdate = 0x8,
    /// CONTINUATION frame — continues a header block split across frames.
    Continuation = 0x9,
}

impl FrameType {
    /// Parse from wire byte.
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

/// 9-byte frame header (RFC 9113 Section 4.1).
///
/// ```text
/// +-----------------------------------------------+
/// |                Length (24)                      |
/// +---------------+---------------+---------------+
/// |  Type (8)     |  Flags (8)    |
/// +-+-------------+---------------+---------------+
/// |R|             Stream Identifier (31)           |
/// +-+---------------------------------------------+
/// ```
#[derive(Debug, Clone, Copy)]
pub struct FrameHeader {
    /// Payload length (24-bit, max 16384 default, up to 16777215).
    pub length: u32,
    /// Frame type.
    pub frame_type: u8,
    /// Type-specific flags.
    pub flags: u8,
    /// Stream identifier (31-bit, R bit must be 0).
    pub stream_id: u32,
}

/// Frame header size in bytes.
pub const FRAME_HEADER_LEN: usize = 9;

impl FrameHeader {
    /// Parse a 9-byte frame header.
    pub fn parse(buf: &[u8; FRAME_HEADER_LEN]) -> Self {
        let length = ((buf[0] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[2] as u32);
        let frame_type = buf[3];
        let flags = buf[4];
        let stream_id = ((buf[5] as u32) << 24)
            | ((buf[6] as u32) << 16)
            | ((buf[7] as u32) << 8)
            | (buf[8] as u32);
        // Clear the R bit (MSB of stream_id).
        let stream_id = stream_id & 0x7FFF_FFFF;

        Self {
            length,
            frame_type,
            flags,
            stream_id,
        }
    }

    /// Serialize to 9 bytes.
    pub fn encode(&self, buf: &mut impl BufMut) {
        buf.put_u8((self.length >> 16) as u8);
        buf.put_u8((self.length >> 8) as u8);
        buf.put_u8(self.length as u8);
        buf.put_u8(self.frame_type);
        buf.put_u8(self.flags);
        buf.put_u32(self.stream_id & 0x7FFF_FFFF);
    }
}

/// A parsed HTTP/2 frame.
#[derive(Debug)]
pub enum Frame {
    /// DATA frame (type 0x0).
    Data(DataFrame),
    /// HEADERS frame (type 0x1).
    Headers(HeadersFrame),
    /// PRIORITY frame (type 0x2).
    Priority(PriorityFrame),
    /// RST_STREAM frame (type 0x3).
    RstStream(RstStreamFrame),
    /// SETTINGS frame (type 0x4).
    Settings(SettingsFrame),
    /// PUSH_PROMISE frame (type 0x5).
    PushPromise(PushPromiseFrame),
    /// PING frame (type 0x6).
    Ping(PingFrame),
    /// GOAWAY frame (type 0x7).
    GoAway(GoAwayFrame),
    /// WINDOW_UPDATE frame (type 0x8).
    WindowUpdate(WindowUpdateFrame),
    /// CONTINUATION frame (type 0x9) — raw header block fragment.
    Continuation {
        /// Stream identifier.
        stream_id: u32,
        /// Whether this is the last CONTINUATION (END_HEADERS set).
        end_headers: bool,
        /// Header block fragment.
        fragment: Bytes,
    },
    /// Unknown frame type — must be ignored per RFC 9113 Section 4.1.
    Unknown {
        /// Frame type byte.
        frame_type: u8,
        /// Flags.
        flags: u8,
        /// Stream ID.
        stream_id: u32,
        /// Payload.
        payload: Bytes,
    },
}

impl Frame {
    /// Parse a frame from header + payload.
    pub fn parse(header: FrameHeader, payload: Bytes) -> Result<Self, crate::h2::H2Error> {
        match FrameType::from_u8(header.frame_type) {
            Some(FrameType::Data) => Ok(Frame::Data(DataFrame::parse(header, payload)?)),
            Some(FrameType::Headers) => Ok(Frame::Headers(HeadersFrame::parse(header, payload)?)),
            Some(FrameType::Priority) => {
                Ok(Frame::Priority(PriorityFrame::parse(header, payload)?))
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
            None => Ok(Frame::Unknown {
                frame_type: header.frame_type,
                flags: header.flags,
                stream_id: header.stream_id,
                payload,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_header_roundtrip() {
        let header = FrameHeader {
            length: 16384,
            frame_type: 0x0, // DATA
            flags: 0x1,      // END_STREAM
            stream_id: 1,
        };
        let mut buf = BytesMut::with_capacity(9);
        header.encode(&mut buf);
        assert_eq!(buf.len(), 9);

        let parsed = FrameHeader::parse(&buf[..9].try_into().unwrap());
        assert_eq!(parsed.length, 16384);
        assert_eq!(parsed.frame_type, 0x0);
        assert_eq!(parsed.flags, 0x1);
        assert_eq!(parsed.stream_id, 1);
    }

    #[test]
    fn frame_header_clears_reserved_bit() {
        let mut raw = [0u8; 9];
        // Set R bit (MSB of stream ID bytes)
        raw[5] = 0x80;
        raw[8] = 0x01;
        let header = FrameHeader::parse(&raw);
        assert_eq!(header.stream_id, 1); // R bit cleared
    }

    #[test]
    fn unknown_frame_types_parse() {
        let header = FrameHeader {
            length: 4,
            frame_type: 0xFF, // unknown
            flags: 0x0,
            stream_id: 0,
        };
        let payload = Bytes::from_static(&[0, 0, 0, 0]);
        let frame = Frame::parse(header, payload).unwrap();
        assert!(matches!(
            frame,
            Frame::Unknown {
                frame_type: 0xFF,
                ..
            }
        ));
    }
}
