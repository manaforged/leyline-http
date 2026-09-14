use bytes::{Buf, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::h2::H2Error;
use crate::h2::frame::{FRAME_HEADER_LEN, Frame, FrameHeader};

use super::DEFAULT_MAX_FRAME_SIZE;

const SLACK: usize = 16 * 1024;

pub struct FrameReader<R> {
    inner: R,
    max_frame_size: u32,
    buf: BytesMut,
    header: Option<FrameHeader>,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            inner: reader,
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
            buf: BytesMut::with_capacity(DEFAULT_MAX_FRAME_SIZE as usize + SLACK),
            header: None,
        }
    }

    pub fn set_max_frame_size(&mut self, size: u32) {
        self.max_frame_size = size;
    }

    pub(crate) fn buffered(&self) -> bool {
        if let Some(header) = &self.header {
            return self.buf.len() >= header.length as usize;
        }
        if self.buf.len() < FRAME_HEADER_LEN {
            return false;
        }
        let mut raw = [0; FRAME_HEADER_LEN];
        raw.copy_from_slice(&self.buf[..FRAME_HEADER_LEN]);
        let header = FrameHeader::parse(&raw);
        header.length > self.max_frame_size
            || self.buf.len() >= FRAME_HEADER_LEN + header.length as usize
    }

    async fn fill(&mut self, want: usize) -> Result<bool, H2Error> {
        while self.buf.len() < want {
            self.buf.reserve(want - self.buf.len() + SLACK);
            let n = self
                .inner
                .read_buf(&mut self.buf)
                .await
                .map_err(H2Error::Io)?;
            if n == 0 {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub async fn next(&mut self) -> Result<Option<Frame>, H2Error> {
        if self.header.is_none() {
            if !self.fill(FRAME_HEADER_LEN).await? {
                if self.buf.is_empty() {
                    return Ok(None);
                }
                return Err(H2Error::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "peer closed mid-frame-header",
                )));
            }
            let mut raw = [0u8; FRAME_HEADER_LEN];
            raw.copy_from_slice(&self.buf[..FRAME_HEADER_LEN]);
            let header = FrameHeader::parse(&raw);
            self.buf.advance(FRAME_HEADER_LEN);
            if header.length > self.max_frame_size {
                return Err(H2Error::FrameTooLarge {
                    size: header.length,
                    max: self.max_frame_size,
                });
            }
            self.header = Some(header);
        }

        let len = self.header.as_ref().map_or(0, |h| h.length) as usize;
        if !self.fill(len).await? {
            return Err(H2Error::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "peer closed mid-frame-payload",
            )));
        }

        let header = self.header.take().expect("header set above");
        let payload = self.buf.split_to(len).freeze();
        Frame::parse(header, payload).map(Some)
    }

    pub fn inner_mut(&mut self) -> &mut R {
        &mut self.inner
    }
}
