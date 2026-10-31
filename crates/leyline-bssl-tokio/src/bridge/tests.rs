use std::io::{self, Read};
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::AsyncStreamBridge;

struct Socket {
    data: Vec<u8>,
    reads: usize,
}

impl AsyncRead for Socket {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.reads += 1;
        let n = buf.remaining().min(self.data.len());
        buf.put_slice(&self.data[..n]);
        self.data.drain(..n);
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for Socket {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[test]
fn a_record_header_and_body_come_from_one_socket_read() {
    let record: Vec<u8> = (0..=255).cycle().take(5 + 1200).collect();
    let mut bridge = AsyncStreamBridge::new(Socket {
        data: record.clone(),
        reads: 0,
    });
    let mut cx = Context::from_waker(std::task::Waker::noop());
    bridge.set_waker(Some(&mut cx));
    let mut header = [0u8; 5];
    let mut body = vec![0u8; 1200];
    bridge.read_exact(&mut header).unwrap();
    bridge.read_exact(&mut body).unwrap();
    assert_eq!([&header[..], &body[..]].concat(), record);
    assert_eq!(bridge.stream.reads, 1);
}
