use std::pin::Pin;
use std::task::{Context, Poll, ready};

use bytes::{Buf, BytesMut};
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::h2::H2Error;

const CAP: usize = 64 * 1024;
const CEILING: usize = 4 * CAP;
const START: usize = 1024;

pub struct FrameWriter {
    buf: BytesMut,
    unflushed: bool,
}

impl FrameWriter {
    pub fn new() -> Self {
        Self {
            buf: BytesMut::with_capacity(START),
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

    pub fn poll_write_some<W: AsyncWrite + Unpin + ?Sized>(
        &mut self,
        io: &mut W,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), H2Error>> {
        if self.buf.is_empty() {
            ready!(Pin::new(&mut *io).poll_flush(cx))?;
            self.unflushed = false;
            return Poll::Ready(Ok(()));
        }
        let n = ready!(Pin::new(&mut *io).poll_write(cx, &self.buf))?;
        if n == 0 {
            return Poll::Ready(Err(
                std::io::Error::from(std::io::ErrorKind::WriteZero).into()
            ));
        }
        self.buf.advance(n);
        self.unflushed = true;
        Poll::Ready(Ok(()))
    }

    pub fn write_preface(&mut self) {
        self.buf
            .extend_from_slice(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
    }

    pub(crate) fn out(&mut self) -> &mut BytesMut {
        &mut self.buf
    }

    pub async fn flush<W: AsyncWrite + Unpin + ?Sized>(
        &mut self,
        io: &mut W,
    ) -> Result<(), H2Error> {
        io.write_all(&self.buf).await?;
        self.buf.clear();
        io.flush().await?;
        self.unflushed = false;
        Ok(())
    }
}
