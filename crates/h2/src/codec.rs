//! Frame codec — reads and writes HTTP/2 frames over async IO.
//!
//! No tokio_util dependency. Just a buffer + AsyncRead/AsyncWrite.

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::ErrorCode;
use crate::frame::{Frame, FrameHeader, FRAME_HEADER_LEN};
use crate::H2Error;

/// Default max frame payload size (RFC 9113 Section 4.2).
const DEFAULT_MAX_FRAME_SIZE: u32 = 16_384;

/// Reads HTTP/2 frames from an async reader.
pub struct FrameReader<R> {
    inner: R,
    max_frame_size: u32,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    /// Create a new frame reader.
    pub fn new(reader: R) -> Self {
        Self {
            inner: reader,
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
        }
    }

    /// Update max frame size (after receiving SETTINGS).
    pub fn set_max_frame_size(&mut self, size: u32) {
        self.max_frame_size = size;
    }

    /// Read the next frame. Returns None on EOF.
    pub async fn next(&mut self) -> Result<Option<Frame>, H2Error> {
        // Read 9-byte header.
        let mut header_buf = [0u8; FRAME_HEADER_LEN];
        match self.inner.read_exact(&mut header_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(H2Error::Io(e)),
        }

        let header = FrameHeader::parse(&header_buf);

        // Validate frame size.
        if header.length > self.max_frame_size {
            return Err(H2Error::FrameTooLarge {
                size: header.length,
                max: self.max_frame_size,
            });
        }

        // Read payload.
        let mut payload = BytesMut::zeroed(header.length as usize);
        if header.length > 0 {
            self.inner.read_exact(&mut payload).await?;
        }

        Frame::parse(header, payload.freeze()).map(Some)
    }

    /// Get a mutable reference to the inner reader.
    pub fn inner_mut(&mut self) -> &mut R {
        &mut self.inner
    }
}

/// Writes HTTP/2 frames to an async writer.
pub struct FrameWriter<W> {
    inner: W,
    buf: BytesMut,
}

impl<W: AsyncWrite + Unpin> FrameWriter<W> {
    /// Create a new frame writer.
    pub fn new(writer: W) -> Self {
        Self {
            inner: writer,
            buf: BytesMut::with_capacity(DEFAULT_MAX_FRAME_SIZE as usize + FRAME_HEADER_LEN),
        }
    }

    /// Write the HTTP/2 client connection preface.
    pub async fn write_preface(&mut self) -> Result<(), H2Error> {
        // RFC 9113 Section 3.4: client connection preface.
        self.inner
            .write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n")
            .await?;
        Ok(())
    }

    /// Write a SETTINGS frame.
    pub async fn write_settings(
        &mut self,
        frame: &crate::frame::SettingsFrame,
    ) -> Result<(), H2Error> {
        self.buf.clear();
        frame.encode(&mut self.buf);
        self.inner.write_all(&self.buf).await?;
        self.inner.flush().await?;
        Ok(())
    }

    /// Write a WINDOW_UPDATE frame.
    pub async fn write_window_update(
        &mut self,
        frame: &crate::frame::WindowUpdateFrame,
    ) -> Result<(), H2Error> {
        self.buf.clear();
        frame.encode(&mut self.buf);
        self.inner.write_all(&self.buf).await?;
        Ok(())
    }

    /// Write a HEADERS frame.
    pub async fn write_headers(
        &mut self,
        frame: &crate::frame::HeadersFrame,
    ) -> Result<(), H2Error> {
        self.buf.clear();
        frame.encode(&mut self.buf);
        self.inner.write_all(&self.buf).await?;
        Ok(())
    }

    /// Write a DATA frame.
    pub async fn write_data(&mut self, frame: &crate::frame::DataFrame) -> Result<(), H2Error> {
        self.buf.clear();
        frame.encode(&mut self.buf);
        self.inner.write_all(&self.buf).await?;
        Ok(())
    }

    /// Write a PING ACK.
    pub async fn write_ping_ack(&mut self, payload: [u8; 8]) -> Result<(), H2Error> {
        let frame = crate::frame::PingFrame { ack: true, payload };
        self.buf.clear();
        frame.encode(&mut self.buf);
        self.inner.write_all(&self.buf).await?;
        Ok(())
    }

    /// Write a SETTINGS ACK.
    pub async fn write_settings_ack(&mut self) -> Result<(), H2Error> {
        let frame = crate::frame::SettingsFrame::ack();
        self.buf.clear();
        frame.encode(&mut self.buf);
        self.inner.write_all(&self.buf).await?;
        Ok(())
    }

    /// Write a RST_STREAM frame.
    pub async fn write_rst_stream(
        &mut self,
        stream_id: u32,
        code: ErrorCode,
    ) -> Result<(), H2Error> {
        let frame = crate::frame::RstStreamFrame {
            stream_id,
            error_code: code,
        };
        self.buf.clear();
        frame.encode(&mut self.buf);
        self.inner.write_all(&self.buf).await?;
        Ok(())
    }

    /// Write a GOAWAY frame.
    pub async fn write_goaway(
        &mut self,
        last_stream_id: u32,
        code: ErrorCode,
    ) -> Result<(), H2Error> {
        let frame = crate::frame::GoAwayFrame {
            last_stream_id,
            error_code: code,
            debug_data: Bytes::new(),
        };
        self.buf.clear();
        frame.encode(&mut self.buf);
        self.inner.write_all(&self.buf).await?;
        Ok(())
    }

    /// Write raw bytes (for CONTINUATION frames).
    pub async fn write_raw(&mut self, data: &[u8]) -> Result<(), H2Error> {
        self.inner.write_all(data).await?;
        Ok(())
    }

    /// Flush the writer.
    pub async fn flush(&mut self) -> Result<(), H2Error> {
        self.inner.flush().await?;
        Ok(())
    }

    /// Get a mutable reference to the inner writer.
    pub fn inner_mut(&mut self) -> &mut W {
        &mut self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::SettingsFrame;

    #[tokio::test]
    async fn write_and_read_settings() {
        let (client, server) = tokio::io::duplex(4096);

        let frame = SettingsFrame {
            ack: false,
            params: vec![(0x1, 65536), (0x2, 0), (0x4, 6291456)],
        };

        // Write.
        let mut writer = FrameWriter::new(client);
        writer.write_settings(&frame).await.unwrap();
        drop(writer); // close write side

        // Read.
        let mut reader = FrameReader::new(server);
        let read_frame = reader.next().await.unwrap().unwrap();

        match read_frame {
            Frame::Settings(s) => {
                assert!(!s.ack);
                assert_eq!(s.params, vec![(0x1, 65536), (0x2, 0), (0x4, 6291456)]);
            }
            _ => panic!("expected Settings frame"),
        }

        // EOF.
        assert!(reader.next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn write_and_read_preface_then_settings() {
        let (client, server) = tokio::io::duplex(4096);

        let mut writer = FrameWriter::new(client);
        writer.write_preface().await.unwrap();
        writer
            .write_settings(&SettingsFrame {
                ack: false,
                params: vec![(0x4, 6291456)],
            })
            .await
            .unwrap();
        drop(writer);

        // Read preface manually.
        let mut reader_raw = server;
        let mut preface = [0u8; 24];
        reader_raw.read_exact(&mut preface).await.unwrap();
        assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");

        // Read settings frame.
        let mut reader = FrameReader::new(reader_raw);
        let frame = reader.next().await.unwrap().unwrap();
        assert!(matches!(frame, Frame::Settings(_)));
    }

    #[tokio::test]
    async fn rejects_oversized_frame() {
        let (client, server) = tokio::io::duplex(4096);

        // Write a frame header claiming 32KB payload (exceeds 16KB default).
        let mut writer = client;
        let header = FrameHeader {
            length: 32768,
            frame_type: 0x0, // DATA
            flags: 0,
            stream_id: 1,
        };
        let mut buf = BytesMut::with_capacity(9);
        header.encode(&mut buf);
        writer.write_all(&buf).await.unwrap();
        drop(writer);

        let mut reader = FrameReader::new(server);
        let result = reader.next().await;
        assert!(matches!(result, Err(H2Error::FrameTooLarge { .. })));
    }
}
