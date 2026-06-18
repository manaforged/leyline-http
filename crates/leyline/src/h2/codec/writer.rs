//! Frame writer — encodes HTTP/2 frames and flushes them through an async writer.

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::h2::error::ErrorCode;
use crate::h2::frame::{
    DataFrame, GoAwayFrame, HeadersFrame, PingFrame, RstStreamFrame, SettingsFrame,
    WindowUpdateFrame, FRAME_HEADER_LEN,
};
use crate::h2::H2Error;

use super::DEFAULT_MAX_FRAME_SIZE;

/// Frames that can be serialized into a `BytesMut` buffer.
///
/// Every concrete frame type already has an inherent `encode(&self, buf: &mut
/// impl BufMut)`; this trait just lets [`FrameWriter::write_frame`] be generic
/// over them while delegating to that existing inherent method (so the emitted
/// bytes are unchanged).
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

    /// Encode `frame` into the reusable buffer and write it out.
    ///
    /// Does **not** flush — callers that need a flush (only `write_settings`)
    /// must call [`FrameWriter::flush`] themselves afterwards. This preserves
    /// the original per-method flush behavior exactly.
    async fn write_frame<F: FrameEncode>(&mut self, frame: &F) -> Result<(), H2Error> {
        self.buf.clear();
        frame.encode(&mut self.buf);
        self.inner.write_all(&self.buf).await?;
        Ok(())
    }

    /// Write a SETTINGS frame.
    pub async fn write_settings(
        &mut self,
        frame: &crate::h2::frame::SettingsFrame,
    ) -> Result<(), H2Error> {
        self.write_frame(frame).await?;
        self.inner.flush().await?;
        Ok(())
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
