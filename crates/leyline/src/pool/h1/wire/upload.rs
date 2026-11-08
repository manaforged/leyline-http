use super::*;

use std::future::poll_fn;
use std::task::{Context, Poll};

use bytes::Buf;
use tokio::io::ReadBuf;

use body::Encoder;

const POLL_BUDGET: usize = 64;
const READ_BUFFER_BYTES: usize = 4096;

type Step = Poll<Result<Option<Sent>, H1Error>>;

pub(super) enum Sent {
    Complete,
    Answered,
    Broken(io::Error),
}

pub(super) struct Upload {
    pub(super) received: Vec<u8>,
    pub(super) sent: Sent,
}

impl Upload {
    pub(super) fn idle(buf: Vec<u8>) -> Self {
        Self {
            received: read_buffer(buf),
            sent: Sent::Complete,
        }
    }
}

fn read_buffer(mut buf: Vec<u8>) -> Vec<u8> {
    buf.clear();
    buf.reserve(READ_BUFFER_BYTES);
    buf
}

struct Pump<'a> {
    stream: &'a mut dyn H1Io,
    queue: VecDeque<Bytes>,
    source: Option<(BoxedStream, Encoder)>,
    received: Vec<u8>,
    reading: bool,
}

pub(super) async fn upload(
    stream: &mut dyn H1Io,
    queue: VecDeque<Bytes>,
    source: Option<(BoxedStream, Encoder)>,
    buf: Vec<u8>,
) -> Result<Upload, H1Error> {
    let mut pump = Pump {
        stream,
        queue,
        source,
        received: read_buffer(buf),
        reading: true,
    };
    let sent = poll_fn(|cx| pump.poll(cx)).await?;
    Ok(Upload {
        received: pump.received,
        sent,
    })
}

impl Pump<'_> {
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<Sent, H1Error>> {
        if self.answered(cx)? {
            return Poll::Ready(Ok(Sent::Answered));
        }
        self.send(cx)
    }

    fn answered(&mut self, cx: &mut Context<'_>) -> Result<bool, H1Error> {
        let mut tmp = [0u8; 2048];
        while self.reading {
            let mut read = ReadBuf::new(&mut tmp);
            match crate::net::probing(true, || {
                Pin::new(&mut *self.stream).poll_read(cx, &mut read)
            }) {
                Poll::Pending => break,
                Poll::Ready(Err(e)) => return Err(e.into()),
                Poll::Ready(Ok(())) if read.filled().is_empty() => {
                    self.reading = false;
                    return Ok(true);
                }
                Poll::Ready(Ok(())) => {
                    self.received.extend_from_slice(read.filled());
                    if answered(&self.received) {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    fn send(&mut self, cx: &mut Context<'_>) -> Poll<Result<Sent, H1Error>> {
        for _ in 0..POLL_BUDGET {
            let step = if self.queue.is_empty() {
                self.pull(cx)
            } else {
                self.write_front(cx)
            };
            match step {
                Poll::Ready(Ok(None)) => {}
                Poll::Ready(Ok(Some(sent))) => return Poll::Ready(Ok(sent)),
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
        cx.waker().wake_by_ref();
        Poll::Pending
    }

    fn write_front(&mut self, cx: &mut Context<'_>) -> Step {
        let Some(front) = self.queue.front_mut() else {
            return Poll::Ready(Ok(None));
        };
        match Pin::new(&mut *self.stream).poll_write(cx, front) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(broken(e).map(Some)),
            Poll::Ready(Ok(0)) => {
                Poll::Ready(Err(io::Error::from(io::ErrorKind::WriteZero).into()))
            }
            Poll::Ready(Ok(n)) => {
                front.advance(n);
                if front.is_empty() {
                    drop(self.queue.pop_front());
                }
                Poll::Ready(Ok(None))
            }
        }
    }

    fn pull(&mut self, cx: &mut Context<'_>) -> Step {
        let Some((source, encoder)) = self.source.as_mut() else {
            return match Pin::new(&mut *self.stream).poll_flush(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Ok(())) => Poll::Ready(Ok(Some(Sent::Complete))),
                Poll::Ready(Err(e)) => Poll::Ready(broken(e).map(Some)),
            };
        };
        match source.poll_next_unpin(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(chunk))) => {
                encoder.encode(chunk, &mut self.queue);
                Poll::Ready(Ok(None))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Err(Error::from_request_body(e).into())),
            Poll::Ready(None) => {
                encoder.finish(&mut self.queue);
                self.source = None;
                Poll::Ready(Ok(None))
            }
        }
    }
}

fn broken(e: io::Error) -> Result<Sent, H1Error> {
    match e.kind() {
        io::ErrorKind::BrokenPipe
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted => Ok(Sent::Broken(e)),
        _ => Err(e.into()),
    }
}

fn answered(buf: &[u8]) -> bool {
    if buf.len() > MAX_H1_HEADER_BYTES {
        return true;
    }
    let mut state = crate::wire::ResponseState::default();
    let mut rest = buf;
    while let Some(end) = find_header_end(rest) {
        let interim = crate::pool::h1::parse::classify(&rest[..end], &mut state).is_ok()
            && state != crate::wire::ResponseState::Final;
        if !interim {
            return true;
        }
        rest = &rest[end + 4..];
    }
    false
}
