//! Frame writer — encodes HTTP/2 frames and flushes them through an async writer.

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::h2::H2Error;
use crate::h2::error::ErrorCode;
use crate::h2::frame::{
    DataFrame, FRAME_HEADER_LEN, GoAwayFrame, HeadersFrame, PingFrame, RstStreamFrame,
    SettingsFrame, WindowUpdateFrame,
};

use super::DEFAULT_MAX_FRAME_SIZE;

/// Frames that can be serialized into a `BytesMut` buffer.
pub(crate) trait FrameEncode {
    /// Serialize this frame (header + payload) into `buf`.
    fn encode(&self, buf: &mut BytesMut);
}

macro_rules! impl_frame_encode {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl FrameEncode for $ty {
                fn encode(&self, buf: &mut BytesMut) {
                    <$ty>::encode(self, buf);
                }
            }
        )+
    };
}

impl_frame_encode!(
    SettingsFrame,
    WindowUpdateFrame,
    HeadersFrame,
    DataFrame,
    PingFrame,
    RstStreamFrame,
    GoAwayFrame,
);

/// Bytes that may accumulate in the outbound buffer before `write_frame` drains it eagerly.
const CAP: usize = 64 * 1024;

/// Writes HTTP/2 frames to an async writer; frames accumulate so one event-loop turn costs one `write_all`, and [`FrameWriter::flush`] must run before awaiting the peer.
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

    /// Buffered bytes not yet handed to the socket.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    /// Write the HTTP/2 client connection preface.
    pub async fn write_preface(&mut self) -> Result<(), H2Error> {
        self.buf
            .extend_from_slice(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
        Ok(())
    }

    /// Append `frame` to the outbound buffer, draining it once it passes `CAP`.
    async fn write_frame<F: FrameEncode>(&mut self, frame: &F) -> Result<(), H2Error> {
        frame.encode(&mut self.buf);
        if self.buf.len() >= CAP {
            self.drain().await?;
        }
        Ok(())
    }

    /// Hand the accumulated bytes to the socket without flushing it.
    async fn drain(&mut self) -> Result<(), H2Error> {
        if !self.buf.is_empty() {
            self.inner.write_all(&self.buf).await?;
            self.buf.clear();
        }
        Ok(())
    }

    /// Write a SETTINGS frame.
    pub async fn write_settings(
        &mut self,
        frame: &crate::h2::frame::SettingsFrame,
    ) -> Result<(), H2Error> {
        self.write_frame(frame).await?;
        self.flush().await
    }

    /// Write a WINDOW_UPDATE frame.
    pub async fn write_window_update(
        &mut self,
        frame: &crate::h2::frame::WindowUpdateFrame,
    ) -> Result<(), H2Error> {
        self.write_frame(frame).await
    }

    /// Write a HEADERS frame.
    pub async fn write_headers(
        &mut self,
        frame: &crate::h2::frame::HeadersFrame,
    ) -> Result<(), H2Error> {
        self.write_frame(frame).await
    }

    /// Write a DATA frame.
    pub async fn write_data(&mut self, frame: &crate::h2::frame::DataFrame) -> Result<(), H2Error> {
        self.write_frame(frame).await
    }

    /// Write a PING ACK.
    pub async fn write_ping_ack(&mut self, payload: [u8; 8]) -> Result<(), H2Error> {
        let frame = crate::h2::frame::PingFrame { ack: true, payload };
        self.write_frame(&frame).await
    }

    /// Write a SETTINGS ACK.
    pub async fn write_settings_ack(&mut self) -> Result<(), H2Error> {
        let frame = crate::h2::frame::SettingsFrame::ack();
        self.write_frame(&frame).await
    }

    /// Write a RST_STREAM frame.
    pub async fn write_rst_stream(
        &mut self,
        stream_id: u32,
        code: ErrorCode,
    ) -> Result<(), H2Error> {
        let frame = crate::h2::frame::RstStreamFrame {
            stream_id,
            error_code: code,
        };
        self.write_frame(&frame).await
    }

    /// Write a GOAWAY frame.
    pub async fn write_goaway(
        &mut self,
        last_stream_id: u32,
        code: ErrorCode,
    ) -> Result<(), H2Error> {
        let frame = crate::h2::frame::GoAwayFrame {
            last_stream_id,
            error_code: code,
            debug_data: Bytes::new(),
        };
        self.write_frame(&frame).await
    }

    /// Write raw bytes (for CONTINUATION frames).
    pub async fn write_raw(&mut self, data: &[u8]) -> Result<(), H2Error> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= CAP {
            self.drain().await?;
        }
        Ok(())
    }

    /// Write the accumulated bytes, then flush the socket.
    pub async fn flush(&mut self) -> Result<(), H2Error> {
        self.drain().await?;
        self.inner.flush().await?;
        Ok(())
    }

    /// Get a mutable reference to the inner writer; flush first or buffered frames are reordered.
    pub fn inner_mut(&mut self) -> &mut W {
        &mut self.inner
    }
}
