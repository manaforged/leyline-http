use std::io;
use std::ops::Range;
use std::pin::Pin;
use std::task::{Context, Poll};

use leyline_bssl::error::ErrorStack;
use leyline_bssl::records::{
    PostHandshake, Records, ALERT, APPLICATION_DATA, HANDSHAKE, HEADER_LEN, MAX_BODY, MAX_PLAINTEXT,
};
use leyline_bssl::ssl::Ssl;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

const INBOUND: usize = HEADER_LEN + MAX_BODY;
const INBOUND_START: usize = 4 * 1024;
const MAX_HANDSHAKE: usize = 64 * 1024;
const RECORDS_PER_WRITE: usize = 4;
const CLOSE_NOTIFY: [u8; 2] = [1, 0];
const USER_CANCELED: u8 = 90;

/// A TLS 1.3 connection whose records leyline seals and opens after the handshake.
pub struct RecordStream<S> {
    stream: S,
    ssl: Ssl,
    records: Records,
    inbound: Box<[u8]>,
    start: usize,
    end: usize,
    plain: Range<usize>,
    handshake: Vec<u8>,
    outbound: Vec<u8>,
    sent: usize,
    eof: bool,
    closing: bool,
}

impl<S> std::fmt::Debug for RecordStream<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordStream")
            .field("records", &self.records)
            .finish_non_exhaustive()
    }
}

fn invalid(error: ErrorStack) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn protocol(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

impl<S> RecordStream<S> {
    pub(crate) fn new(stream: S, ssl: Ssl, records: Records, unread: &[u8]) -> Self {
        let mut inbound = vec![0; INBOUND_START.max(unread.len())].into_boxed_slice();
        inbound[..unread.len()].copy_from_slice(unread);
        Self {
            stream,
            ssl,
            records,
            inbound,
            start: 0,
            end: unread.len(),
            plain: 0..0,
            handshake: Vec::new(),
            outbound: Vec::new(),
            sent: 0,
            eof: false,
            closing: false,
        }
    }

    /// Returns the SSL whose handshake these records follow.
    #[must_use]
    pub fn ssl(&self) -> &Ssl {
        &self.ssl
    }

    /// Returns the underlying stream.
    #[must_use]
    pub fn get_ref(&self) -> &S {
        &self.stream
    }

    /// Moves the record state onto the stream `f` makes from the current one.
    pub fn try_map_io<T, E>(self, f: impl FnOnce(S) -> Result<T, E>) -> Result<RecordStream<T>, E> {
        Ok(RecordStream {
            stream: f(self.stream)?,
            ssl: self.ssl,
            records: self.records,
            inbound: self.inbound,
            start: self.start,
            end: self.end,
            plain: self.plain,
            handshake: self.handshake,
            outbound: self.outbound,
            sent: self.sent,
            eof: self.eof,
            closing: self.closing,
        })
    }

    fn next_record(&mut self) -> io::Result<bool> {
        let buffered = &self.inbound[self.start..self.end];
        if buffered.len() < HEADER_LEN {
            return Ok(false);
        }
        if buffered[0] != APPLICATION_DATA {
            return Err(protocol(
                "TLS 1.3 record has an outer type other than application data",
            ));
        }
        let body = usize::from(u16::from_be_bytes([buffered[3], buffered[4]]));
        if body > MAX_BODY {
            return Err(protocol("TLS record exceeds the maximum length"));
        }
        let end = self.start + HEADER_LEN + body;
        if end > self.end {
            return Ok(false);
        }
        let record = &mut self.inbound[self.start..end];
        let (kind, range) = self.records.open(record).map_err(invalid)?;
        let plain = self.start + range.start..self.start + range.end;
        self.start = end;
        match kind {
            APPLICATION_DATA => self.plain = plain,
            HANDSHAKE => {
                self.handshake.extend_from_slice(&self.inbound[plain]);
                self.post_handshake()?;
            }
            ALERT => match &self.inbound[plain] {
                [_, 0] => self.eof = true,
                [_, USER_CANCELED] => {}
                [_, code] => {
                    return Err(protocol(&format!("TLS peer sent fatal alert {code}")));
                }
                _ => return Err(protocol("TLS alert record has the wrong length")),
            },
            _ => return Err(protocol("TLS record has an unknown content type")),
        }
        Ok(true)
    }

    fn post_handshake(&mut self) -> io::Result<()> {
        if self.handshake.len() > MAX_HANDSHAKE {
            return Err(protocol("TLS post-handshake message exceeds 64 KiB"));
        }
        while self.handshake.len() >= 4 {
            let len = usize::from(self.handshake[1]) << 16
                | usize::from(self.handshake[2]) << 8
                | usize::from(self.handshake[3]);
            if self.handshake.len() < 4 + len {
                break;
            }
            let message: Vec<u8> = self.handshake.drain(..4 + len).collect();
            if self
                .records
                .post_handshake(&self.ssl, &message)
                .map_err(invalid)?
                == PostHandshake::UpdateRequested
            {
                self.records
                    .update_write(&mut self.outbound)
                    .map_err(invalid)?;
            }
        }
        Ok(())
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> RecordStream<S> {
    fn poll_send(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.sent < self.outbound.len() {
            let n = std::task::ready!(
                Pin::new(&mut self.stream).poll_write(cx, &self.outbound[self.sent..])
            )?;
            if n == 0 {
                return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
            }
            self.sent += n;
        }
        self.outbound.clear();
        self.sent = 0;
        Poll::Ready(Ok(()))
    }

    fn poll_fill(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<usize>> {
        if self.start > 0 {
            self.inbound.copy_within(self.start..self.end, 0);
            self.end -= self.start;
            self.start = 0;
        }
        if self.end == self.inbound.len() {
            let mut grown = vec![0; INBOUND.max(2 * self.inbound.len())].into_boxed_slice();
            grown[..self.end].copy_from_slice(&self.inbound[..self.end]);
            self.inbound = grown;
        }
        let mut buf = ReadBuf::new(&mut self.inbound[self.end..]);
        std::task::ready!(Pin::new(&mut self.stream).poll_read(cx, &mut buf))?;
        let n = buf.filled().len();
        self.end += n;
        Poll::Ready(Ok(n))
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for RecordStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if !this.outbound.is_empty() {
            if let Poll::Ready(result) = this.poll_send(cx) {
                result?;
            }
        }
        let before = buf.filled().len();
        loop {
            if !this.plain.is_empty() {
                let n = this.plain.len().min(buf.remaining());
                buf.put_slice(&this.inbound[this.plain.start..this.plain.start + n]);
                this.plain.start += n;
                if buf.remaining() == 0 {
                    return Poll::Ready(Ok(()));
                }
                continue;
            }
            if this.eof {
                return Poll::Ready(Ok(()));
            }
            if this.next_record()? {
                continue;
            }
            if buf.filled().len() > before {
                return Poll::Ready(Ok(()));
            }
            match this.poll_fill(cx)? {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(0) if this.start == this.end => {
                    this.eof = true;
                    return Poll::Ready(Ok(()));
                }
                Poll::Ready(0) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "TLS connection closed inside a record",
                    )));
                }
                Poll::Ready(_) => {}
            }
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for RecordStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.poll_send(cx)?.is_pending() {
            return Poll::Pending;
        }
        let take = buf.len().min(MAX_PLAINTEXT * RECORDS_PER_WRITE);
        for chunk in buf[..take].chunks(MAX_PLAINTEXT) {
            this.records
                .seal(APPLICATION_DATA, chunk, &mut this.outbound)
                .map_err(invalid)?;
        }
        if let Poll::Ready(result) = this.poll_send(cx) {
            result?;
        }
        Poll::Ready(Ok(take))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        std::task::ready!(this.poll_send(cx))?;
        Pin::new(&mut this.stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if !this.closing {
            this.closing = true;
            this.records
                .seal(ALERT, &CLOSE_NOTIFY, &mut this.outbound)
                .map_err(invalid)?;
        }
        std::task::ready!(this.poll_send(cx))?;
        Pin::new(&mut this.stream).poll_shutdown(cx)
    }
}
