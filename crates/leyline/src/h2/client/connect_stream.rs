use std::io;
use std::pin::Pin;
use std::sync::atomic::Ordering;

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::PollSender;

use crate::h2::connection::{HeaderPair, PseudoHeaders};
use crate::h2::error::{ErrorCode, H2Error};

use super::driver::{DriverCommand, ResponseSink, STREAM_REQ_BODY_CAPACITY};
use super::handle::H2Client;
use super::types::H2ResponseEx;

pub struct H2ConnectStream {
    pub(super) status: u16,
    pub(super) response_headers: Vec<(String, String)>,
    pub(super) write_tx: Option<PollSender<io::Result<Bytes>>>,
    pub(super) read_rx: mpsc::Receiver<io::Result<Bytes>>,
    pub(super) read_leftover: Bytes,
    pub(super) read_eof: bool,
    pub(super) shutdown_state: ShutdownState,
}

pub(super) enum ShutdownState {
    Open,
    Draining,
}

impl std::fmt::Debug for H2ConnectStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("H2ConnectStream")
            .field("status", &self.status)
            .field("response_headers_len", &self.response_headers.len())
            .field("write_open", &self.write_tx.is_some())
            .field("read_eof", &self.read_eof)
            .finish()
    }
}

impl H2ConnectStream {
    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn response_headers(&self) -> &[(String, String)] {
        &self.response_headers
    }
}

impl tokio::io::AsyncRead for H2ConnectStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        use std::task::Poll;
        if !self.read_leftover.is_empty() {
            let take = self.read_leftover.len().min(buf.remaining());
            let chunk = self.read_leftover.slice(0..take);
            buf.put_slice(&chunk);
            self.read_leftover = self.read_leftover.slice(take..);
            return Poll::Ready(Ok(()));
        }
        if self.read_eof {
            return Poll::Ready(Ok(()));
        }
        match self.read_rx.poll_recv(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                if bytes.is_empty() {
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                let take = bytes.len().min(buf.remaining());
                buf.put_slice(&bytes[..take]);
                if take < bytes.len() {
                    self.read_leftover = bytes.slice(take..);
                }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Err(e)),
            Poll::Ready(None) => {
                self.read_eof = true;
                Poll::Ready(Ok(()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl tokio::io::AsyncWrite for H2ConnectStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        use std::task::Poll;
        let this = self.get_mut();
        let tx = match this.write_tx.as_mut() {
            Some(t) => t,
            None => {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "H2ConnectStream write half closed",
                )));
            }
        };
        match tx.poll_reserve(cx) {
            Poll::Ready(Ok(())) => {
                let n = buf.len();
                if tx.send_item(Ok(Bytes::copy_from_slice(buf))).is_err() {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "H2 driver dropped the CONNECT stream",
                    )));
                }
                Poll::Ready(Ok(n))
            }
            Poll::Ready(Err(_closed)) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "H2 driver dropped the CONNECT stream",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        use std::task::Poll;
        if matches!(self.shutdown_state, ShutdownState::Open) {
            self.write_tx = None;
            self.shutdown_state = ShutdownState::Draining;
        }

        if self.read_eof {
            return Poll::Ready(Ok(()));
        }

        for _ in 0..H2_CONNECT_SHUTDOWN_DRAIN_BUDGET {
            match self.read_rx.poll_recv(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    if bytes.is_empty() {
                        continue;
                    }
                    if self.read_leftover.len() >= H2_CONNECT_LEFTOVER_CAP {
                        self.read_eof = true;
                        return Poll::Ready(Err(io::Error::other(
                            "H2ConnectStream shutdown drain exceeded \
                             H2_CONNECT_LEFTOVER_CAP; peer streamed past \
                             shutdown faster than the caller drained",
                        )));
                    }
                    if self.read_leftover.is_empty() {
                        self.read_leftover = bytes;
                    } else {
                        let mut buf =
                            bytes::BytesMut::with_capacity(self.read_leftover.len() + bytes.len());
                        buf.extend_from_slice(&self.read_leftover);
                        buf.extend_from_slice(&bytes);
                        self.read_leftover = buf.freeze();
                    }
                }
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(e)),
                Poll::Ready(None) => {
                    self.read_eof = true;
                    return Poll::Ready(Ok(()));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

const H2_CONNECT_LEFTOVER_CAP: usize = 1 << 20;
const H2_CONNECT_SHUTDOWN_DRAIN_BUDGET: usize = 64;

impl Drop for H2ConnectStream {
    fn drop(&mut self) {
        self.write_tx = None;
    }
}

impl H2Client {
    pub fn peer_enables_connect_protocol(&self) -> bool {
        self.peer_settings.enable_connect_protocol()
    }

    pub async fn open_extended_connect(
        &self,
        pseudo: PseudoHeaders,
        headers: Vec<HeaderPair>,
    ) -> Result<H2ConnectStream, H2Error> {
        if !self.peer_enables_connect_protocol() {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "peer did not advertise SETTINGS_ENABLE_CONNECT_PROTOCOL=1; \
                         fall back to the HTTP/1.1 path"
                    .into(),
            });
        }
        if self.closed.load(Ordering::Acquire) {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "connection closed".into(),
            });
        }
        if !pseudo.method.eq_ignore_ascii_case("CONNECT") || pseudo.protocol.is_none() {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "extended CONNECT requires :method=CONNECT and :protocol".into(),
            });
        }

        let (write_tx, write_rx) = mpsc::channel::<io::Result<Bytes>>(STREAM_REQ_BODY_CAPACITY);
        let (headers_tx, headers_rx) = oneshot::channel::<Result<H2ResponseEx, H2Error>>();
        let (sink, body_rx) = ResponseSink::streaming(headers_tx);

        let cmd = DriverCommand::OpenConnect {
            pseudo,
            headers,
            write_rx,
            sink,
        };
        self.tx.send(cmd).await.map_err(|e| {
            tracing::debug!(error = %e, "h2 driver command channel closed");
            H2Error::Stream {
                stream_id: 0,
                code: ErrorCode::RefusedStream,
            }
        })?;

        let resp = match headers_rx.await {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(H2Error::Connection {
                    code: ErrorCode::NoError,
                    reason: "driver dropped response sender".into(),
                });
            }
        };

        Ok(H2ConnectStream {
            shutdown_state: ShutdownState::Open,
            status: resp.status,
            response_headers: resp
                .headers
                .into_iter()
                .map(|(k, v)| (k.as_str().to_owned(), v.as_str().to_owned()))
                .collect(),
            write_tx: Some(PollSender::new(write_tx)),
            read_rx: body_rx,
            read_leftover: Bytes::new(),
            read_eof: false,
        })
    }
}

#[cfg(test)]
mod connect_stream_tests;
