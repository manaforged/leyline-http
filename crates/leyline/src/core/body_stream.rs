use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use bytes::Bytes;
use futures_util::Stream;
use tokio::sync::mpsc;
use tokio::time::Sleep;
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

use crate::core::config::HostPass;

use crate::core::session::decompress::Decoder;
use crate::trace::BodyWatch;

pub struct BodyStream {
    rx: mpsc::Receiver<std::io::Result<Bytes>>,
    read_timeout: Option<Duration>,
    idle: Option<Pin<Box<Sleep>>>,
    body_deadline: Option<Pin<Box<Sleep>>>,
    decoded: Option<Box<Decoded>>,
    watch: Option<BodyWatch>,
    shutdown: Option<Pin<Box<WaitForCancellationFutureOwned>>>,
    shut: bool,
    ended: bool,
    pass: Option<HostPass>,
}

struct Decoded {
    raw: BodyStream,
    decoder: Decoder,
    done: bool,
}

impl Decoded {
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Option<std::io::Result<Bytes>>> {
        loop {
            if self.done {
                return Poll::Ready(None);
            }
            let mut out = Vec::new();
            let step = match Pin::new(&mut self.raw).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Err(e))) => {
                    self.done = true;
                    return Poll::Ready(Some(Err(e)));
                }
                Poll::Ready(Some(Ok(chunk))) => self.decoder.feed(&chunk, &mut out),
                Poll::Ready(None) => {
                    self.done = true;
                    self.decoder.finish(&mut out)
                }
            };
            if let Err(e) = step {
                self.done = true;
                return Poll::Ready(Some(Err(e.into_io())));
            }
            if !out.is_empty() {
                return Poll::Ready(Some(Ok(Bytes::from(out))));
            }
        }
    }
}

impl BodyStream {
    pub(crate) fn new(rx: mpsc::Receiver<std::io::Result<Bytes>>) -> Self {
        Self {
            rx,
            read_timeout: None,
            idle: None,
            body_deadline: None,
            decoded: None,
            watch: None,
            shutdown: None,
            shut: false,
            ended: false,
            pass: None,
        }
    }

    pub(crate) fn decoded(mut raw: BodyStream, decoder: Decoder) -> Self {
        let (_, rx) = mpsc::channel(1);
        let watch = raw.watch.take();
        Self {
            watch,
            decoded: Some(Box::new(Decoded {
                raw,
                decoder,
                done: false,
            })),
            ..Self::new(rx)
        }
    }

    pub(crate) fn from_bytes(buf: Bytes) -> Self {
        let (tx, rx) = mpsc::channel(1);
        if !buf.is_empty() {
            tx.try_send(Ok(buf))
                .expect("fresh channel with capacity 1 has room");
        }
        drop(tx);
        Self::new(rx)
    }

    pub(crate) fn set_read_timeout(&mut self, timeout: Option<Duration>) {
        self.read_timeout = timeout;
    }

    pub(crate) fn set_body_timeout(&mut self, timeout: Option<Duration>) {
        self.body_deadline = timeout.map(|limit| Box::pin(tokio::time::sleep(limit)));
    }

    fn poll_body_deadline(&mut self, cx: &mut Context<'_>) -> Poll<std::io::Error> {
        let Some(sleep) = self.body_deadline.as_mut() else {
            return Poll::Pending;
        };
        ready!(sleep.as_mut().poll(cx));
        self.body_deadline = None;
        self.ended = true;
        self.pass = None;
        Poll::Ready(timed_out(
            "body timeout: response body not complete within the configured body timeout",
        ))
    }

    pub(crate) fn stop_on(&mut self, token: &CancellationToken) {
        self.shutdown = Some(Box::pin(token.clone().cancelled_owned()));
    }

    pub(crate) fn hold(&mut self, pass: Option<HostPass>) {
        self.pass = pass;
    }

    fn poll_shutdown(&mut self, cx: &mut Context<'_>) -> bool {
        if !self.shut
            && let Some(wait) = self.shutdown.as_mut()
            && wait.as_mut().poll(cx).is_ready()
        {
            self.shutdown = None;
            self.shut = true;
            self.pass = None;
        }
        self.shut
    }

    pub(crate) fn watch_end(&mut self) {
        self.watch = BodyWatch::begin();
    }

    fn poll_raw(&mut self, cx: &mut Context<'_>) -> Poll<Option<std::io::Result<Bytes>>> {
        match self.rx.poll_recv(cx) {
            Poll::Ready(item) => {
                self.idle = None;
                Poll::Ready(item)
            }
            Poll::Pending => {
                let Some(timeout) = self.read_timeout else {
                    return Poll::Pending;
                };
                let sleep = self
                    .idle
                    .get_or_insert_with(|| Box::pin(tokio::time::sleep(timeout)));
                match sleep.as_mut().poll(cx) {
                    Poll::Ready(()) => {
                        self.idle = None;
                        Poll::Ready(Some(Err(timed_out(
                            "read timeout: no response body chunk within the configured read_timeout",
                        ))))
                    }
                    Poll::Pending => Poll::Pending,
                }
            }
        }
    }
}

fn timed_out(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::TimedOut, message)
}

impl Stream for BodyStream {
    type Item = std::io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.ended {
            return Poll::Ready(None);
        }
        if this.poll_shutdown(cx) {
            return Poll::Ready(Some(Err(std::io::Error::other(
                crate::core::session::execute::shut_down(),
            ))));
        }
        if let Poll::Ready(err) = this.poll_body_deadline(cx) {
            return Poll::Ready(Some(Err(err)));
        }
        let polled = match this.decoded.as_mut() {
            Some(decoded) => decoded.poll(cx),
            None => this.poll_raw(cx),
        };
        if let (Poll::Ready(item), Some(watch)) = (&polled, this.watch.as_mut()) {
            watch.observe(item.as_ref());
        }
        if matches!(polled, Poll::Ready(None | Some(Err(_)))) {
            this.pass = None;
        }
        polled
    }
}

impl std::fmt::Debug for BodyStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BodyStream").finish_non_exhaustive()
    }
}
