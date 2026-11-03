use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use bytes::{Buf, BytesMut};
use tokio::io::AsyncRead;

use crate::h2::H2Error;
use crate::h2::frame::{FRAME_HEADER_LEN, Frame, FrameHeader};

use crate::h2::config::DEFAULT_MAX_FRAME_SIZE;

pub(super) const SLACK: usize = 512;
pub(super) const BULK_AT: usize = 4 * 1024;
pub(super) const BULK_SLACK: usize = 16 * 1024;

pub struct FrameReader {
    max_frame_size: u32,
    buf: BytesMut,
    header: Option<FrameHeader>,
}

impl Default for FrameReader {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameReader {
    pub fn new() -> Self {
        Self {
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
            buf: BytesMut::with_capacity(SLACK),
            header: None,
        }
    }

    pub fn set_max_frame_size(&mut self, size: u32) {
        self.max_frame_size = size;
    }

    #[cfg(test)]
    pub(super) fn read_capacity(&self) -> usize {
        self.buf.capacity()
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

    fn poll_fill<R: AsyncRead + Unpin + ?Sized>(
        &mut self,
        io: &mut R,
        cx: &mut Context<'_>,
        want: usize,
    ) -> Poll<Result<bool, H2Error>> {
        while self.buf.len() < want {
            let slack = if want > BULK_AT { BULK_SLACK } else { SLACK };
            self.buf.reserve(want - self.buf.len() + slack);
            let n = ready!(tokio_util::io::poll_read_buf(
                Pin::new(&mut *io),
                cx,
                &mut self.buf
            ))
            .map_err(H2Error::Io)?;
            if n == 0 {
                return Poll::Ready(Ok(false));
            }
        }
        Poll::Ready(Ok(true))
    }

    pub fn poll_next<R: AsyncRead + Unpin + ?Sized>(
        &mut self,
        io: &mut R,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<Frame>, H2Error>> {
        if self.header.is_none() {
            if !ready!(self.poll_fill(io, cx, FRAME_HEADER_LEN))? {
                if self.buf.is_empty() {
                    return Poll::Ready(Ok(None));
                }
                return Poll::Ready(Err(H2Error::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "peer closed mid-frame-header",
                ))));
            }
            let mut raw = [0u8; FRAME_HEADER_LEN];
            raw.copy_from_slice(&self.buf[..FRAME_HEADER_LEN]);
            let header = FrameHeader::parse(&raw);
            self.buf.advance(FRAME_HEADER_LEN);
            if header.length > self.max_frame_size {
                return Poll::Ready(Err(H2Error::FrameTooLarge {
                    size: header.length,
                    max: self.max_frame_size,
                }));
            }
            self.header = Some(header);
        }

        let len = self.header.as_ref().map_or(0, |h| h.length) as usize;
        if !ready!(self.poll_fill(io, cx, len))? {
            return Poll::Ready(Err(H2Error::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "peer closed mid-frame-payload",
            ))));
        }

        let header = self.header.take().expect("header set above");
        let payload = self.buf.split_to(len).freeze();
        Poll::Ready(Frame::parse(header, payload).map(Some))
    }

    pub fn next<R: AsyncRead + Unpin + ?Sized>(
        &mut self,
        io: &mut R,
    ) -> impl Future<Output = Result<Option<Frame>, H2Error>> {
        poll_fn(move |cx| self.poll_next(io, cx))
    }
}
