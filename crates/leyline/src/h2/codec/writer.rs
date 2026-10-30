use bytes::{Buf, BytesMut};
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::h2::H2Error;
use crate::h2::config::DEFAULT_MAX_FRAME_SIZE;
use crate::h2::frame::{Encode, FRAME_HEADER_LEN, SettingsFrame};

const CAP: usize = 64 * 1024;
const CEILING: usize = 4 * CAP;

pub struct FrameWriter<W> {
    inner: W,
    buf: BytesMut,
    unflushed: bool,
}

impl<W: AsyncWrite + Unpin> FrameWriter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            inner: writer,
            buf: BytesMut::with_capacity(DEFAULT_MAX_FRAME_SIZE as usize + FRAME_HEADER_LEN),
            unflushed: false,
        }
    }

    pub fn congested(&self) -> bool {
        self.buf.len() >= CAP
    }

    pub fn allowance(&self) -> usize {
        CAP.saturating_sub(self.buf.len())
    }

    pub fn saturated(&self) -> bool {
        self.buf.len() >= CEILING
    }

    pub fn has_output(&self) -> bool {
        !self.buf.is_empty() || self.unflushed
    }

    pub async fn write_some(&mut self) -> Result<(), H2Error> {
        if self.buf.is_empty() {
            self.inner.flush().await?;
            self.unflushed = false;
            return Ok(());
        }
        let n = self.inner.write(&self.buf).await?;
        if n == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::WriteZero).into());
        }
        self.buf.advance(n);
        self.unflushed = true;
        Ok(())
    }

    pub async fn write_preface(&mut self) -> Result<(), H2Error> {
        self.buf
            .extend_from_slice(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
        Ok(())
    }

    pub(crate) async fn write_frame<F: Encode>(&mut self, frame: &F) -> Result<(), H2Error> {
        frame.encode(&mut self.buf);
        Ok(())
    }

    pub async fn write_settings(&mut self, frame: &SettingsFrame) -> Result<(), H2Error> {
        self.write_frame(frame).await?;
        self.flush().await
    }

    pub async fn flush(&mut self) -> Result<(), H2Error> {
        self.inner.write_all(&self.buf).await?;
        self.buf.clear();
        self.inner.flush().await?;
        self.unflushed = false;
        Ok(())
    }
}
