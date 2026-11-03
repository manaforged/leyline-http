use std::future::poll_fn;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::oneshot;

use crate::h2::error::H2Error;
use crate::h2::frame::Frame;

use super::*;
use crate::core::deadline::Tick;

enum Event {
    Written(Result<(), H2Error>),
    Read(Result<Option<Frame>, H2Error>),
    Command(Option<DriverCommand>),
    BodyChunk(BodyChunkIn),
    Ping(oneshot::Sender<()>),
    Sweep,
    FlushStalled,
}

pub(super) enum Io {
    Written(Result<(), H2Error>),
    Read(Result<Option<Frame>, H2Error>),
}

/// Polls a pending write when `writing`, then a frame read when `reading`, on the one connection both use.
pub(super) fn poll_io<T: AsyncRead + AsyncWrite + Unpin>(
    io: &mut T,
    writer: &mut FrameWriter,
    reader: &mut FrameReader,
    writing: bool,
    reading: bool,
    cx: &mut Context<'_>,
) -> Poll<Io> {
    if writing && let Poll::Ready(written) = writer.poll_write_some(io, cx) {
        return Poll::Ready(Io::Written(written));
    }
    if reading && let Poll::Ready(frame) = reader.poll_next(io, cx) {
        return Poll::Ready(Io::Read(frame));
    }
    Poll::Pending
}

fn poll_armed<M>(
    rx: &mut tokio::sync::mpsc::Receiver<M>,
    armed: &mut bool,
    cx: &mut Context<'_>,
) -> Poll<Option<M>> {
    if *armed && rx.is_empty() {
        return Poll::Pending;
    }
    let polled = rx.poll_recv(cx);
    *armed = polled.is_pending();
    polled
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) async fn wait_event(
        &mut self,
        sweep_tick: &mut Tick,
        flush_tick: &mut Tick,
    ) -> Result<Option<bool>, H2Error> {
        let event = self.next_event(sweep_tick, flush_tick).await;
        if !self.pending.is_empty()
            && self.writer.has_output()
            && !matches!(event, Event::Written(_))
        {
            self.output.defer();
        }
        self.on_event(event).await
    }

    async fn next_event(&mut self, sweep_tick: &mut Tick, flush_tick: &mut Tick) -> Event {
        let writing = self.writer.has_output();
        let reading = !self.writer.saturated();
        let accepting = self.accepting_commands();
        let taking_body = !self.writer.congested()
            && self
                .body_chunks
                .as_ref()
                .is_some_and(|(tx, rx)| tx.strong_count() > 1 || !rx.is_empty());
        let stalled = self.stalled > 0;
        let write_first = self.pending.is_empty() || self.output.batch_full();
        poll_fn(|cx| {
            if sweep_tick.poll_tick(cx).is_ready() {
                return Poll::Ready(Event::Sweep);
            }
            if let Poll::Ready(io) = poll_io(
                &mut self.io,
                &mut self.writer,
                &mut self.reader,
                writing && write_first,
                reading,
                cx,
            ) {
                return Poll::Ready(match io {
                    Io::Written(written) => Event::Written(written),
                    Io::Read(frame) => Event::Read(frame),
                });
            }
            if accepting && let Poll::Ready(cmd) = self.command_rx.poll_recv(cx) {
                return Poll::Ready(Event::Command(cmd));
            }
            if taking_body
                && let Some((_, rx)) = self.body_chunks.as_mut()
                && let Poll::Ready(Some(chunk)) = rx.poll_recv(cx)
            {
                return Poll::Ready(Event::BodyChunk(chunk));
            }
            if let Poll::Ready(Some(ack_tx)) =
                poll_armed(&mut self.ping_rx, &mut self.ping_armed, cx)
            {
                return Poll::Ready(Event::Ping(ack_tx));
            }
            if writing
                && !write_first
                && let Poll::Ready(written) = self.writer.poll_write_some(&mut self.io, cx)
            {
                return Poll::Ready(Event::Written(written));
            }
            if stalled && flush_tick.poll_tick(cx).is_ready() {
                return Poll::Ready(Event::FlushStalled);
            }
            Poll::Pending
        })
        .await
    }

    async fn on_event(&mut self, event: Event) -> Result<Option<bool>, H2Error> {
        match event {
            Event::Written(written) => self.output.wrote(written)?,
            Event::Read(frame) => {
                return Ok(self.on_read(frame).await?.is_continue().then_some(false));
            }
            Event::Command(cmd) => self.on_command_received(cmd)?,
            Event::BodyChunk(chunk) => self.on_body_chunk(chunk)?,
            Event::Ping(ack_tx) => self.on_command(DriverCommand::Ping { ack_tx })?,
            Event::Sweep => return Ok(Some(true)),
            Event::FlushStalled => self.flush_stalled()?,
        }
        Ok(Some(false))
    }
}
