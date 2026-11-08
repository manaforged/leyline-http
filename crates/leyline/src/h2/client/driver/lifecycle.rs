use std::ops::ControlFlow;
use std::sync::atomic::Ordering;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::h2::error::ErrorCode;

use crate::h2::H2Error;
use crate::h2::stream_state::{StreamEvent, StreamState};

use super::*;
use crate::deadline::Tick;
use crate::h2::frame::U31_MAX;

pub(super) enum Admit {
    Open,
    Defer,
    Refuse(H2Error),
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) async fn run(mut self) -> Result<(), H2Error> {
        let result = self.event_loop().await;
        self.closed.store(true, Ordering::Release);
        let final_err = match &result {
            Ok(()) => H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "connection closed".into(),
            },
            Err(e) => e.duplicate(),
        };
        for (_, mut actor) in self.streams.drain() {
            if actor.remote_done {
                actor.deliver_ok();
            } else {
                actor.deliver_err(final_err.duplicate());
            }
        }
        while let Some(cmd) = self.pending.pop_front() {
            if let Some(sink) = cmd.into_sink() {
                sink.fail(final_err.duplicate());
            }
        }
        while let Ok(cmd) = self.command_rx.try_recv() {
            if let Some(sink) = cmd.into_sink() {
                sink.fail(final_err.duplicate());
            }
        }
        result
    }

    pub(super) async fn event_loop(&mut self) -> Result<(), H2Error> {
        let mut sweep_tick = Tick::every(SWEEP_PERIOD);
        let mut flush_tick = Tick::every(FLUSH_PERIOD);
        loop {
            if self.shutdown_started && self.streams.is_empty() && self.goaway_drained() {
                return Ok(());
            }
            if self.read_buffered_frames().await?.is_break() {
                return Ok(());
            }
            self.run_ready_work()?;
            if self
                .output
                .coalesce(self.writer.has_output() && self.streams.len() > 1)
            {
                tokio::task::yield_now().await;
                continue;
            }
            let Some(tick_fired) = self.wait_event(&mut sweep_tick, &mut flush_tick).await? else {
                return Ok(());
            };
            self.after_event(tick_fired)?;
        }
    }

    fn run_ready_work(&mut self) -> Result<(), H2Error> {
        self.take_queued_commands();
        self.take_queued_body_chunks()?;
        self.drain_pending();
        self.try_drain_pending();
        Ok(())
    }

    fn after_event(&mut self, tick_fired: bool) -> Result<(), H2Error> {
        self.take_queued_commands();
        self.try_drain_pending();
        if tick_fired {
            self.check_ping_deadlines()?;
            self.sweep_cancelled_streams()?;
            self.check_write_stall()?;
        }
        Ok(())
    }

    pub(super) fn on_command_received(&mut self, cmd: Option<DriverCommand>) {
        match cmd {
            Some(cmd) => self.on_command(cmd),
            None => self.shutdown_started = true,
        }
    }

    async fn read_buffered_frames(&mut self) -> Result<ControlFlow<()>, H2Error> {
        for _ in 0..128 {
            if !self.reader.buffered() || self.writer.saturated() {
                break;
            }
            let frame = self.reader.next(&mut self.io).await;
            if self.on_read(frame).await?.is_break() {
                return Ok(ControlFlow::Break(()));
            }
        }
        Ok(ControlFlow::Continue(()))
    }

    pub(super) async fn on_read(
        &mut self,
        frame: Result<Option<Frame>, H2Error>,
    ) -> Result<ControlFlow<()>, H2Error> {
        let frame = match frame {
            Err(err @ H2Error::Stream { stream_id, code }) => {
                self.reset_stream(stream_id, err, code)?;
                return Ok(ControlFlow::Continue(()));
            }
            other => other?,
        };
        let Some(frame) = frame else {
            if self.shutdown_started {
                return Ok(ControlFlow::Break(()));
            }
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "peer closed connection".into(),
            });
        };
        self.on_inbound_frame(frame).await?;
        self.drain_pending();
        Ok(ControlFlow::Continue(()))
    }

    fn take_queued_body_chunks(&mut self) -> Result<(), H2Error> {
        let queued = self.body_chunks.as_ref().map_or(0, |(_, rx)| rx.len());
        for _ in 0..queued {
            if self.writer.congested() {
                break;
            }
            let Some((_, rx)) = self.body_chunks.as_mut() else {
                break;
            };
            let Ok(chunk) = rx.try_recv() else {
                break;
            };
            self.on_body_chunk(chunk)?;
        }
        Ok(())
    }

    pub(super) fn accepting_commands(&self) -> bool {
        !self.shutdown_started
            && self.pending.is_empty()
            && (!self.writer.congested() || self.output.stalled())
    }

    fn take_queued_commands(&mut self) {
        while let Ok(ping) = self.ping_rx.try_recv() {
            self.on_command(DriverCommand::Ping(ping));
        }
        for _ in 0..self.command_rx.len() {
            if !self.pending.is_empty() || self.writer.congested() {
                break;
            }
            let Ok(next) = self.command_rx.try_recv() else {
                break;
            };
            self.on_command(next);
        }
    }

    pub(super) fn send_ping(&mut self, (ack_tx, deadline): Ping) {
        self.ping_seq = self.ping_seq.wrapping_add(1);
        let payload = self.ping_seq.to_be_bytes();
        PingFrame {
            ack: false,
            payload,
        }
        .encode(self.writer.out());
        self.pings.push_back((payload, ack_tx, deadline));
    }

    pub(super) fn sweep_cancelled_streams(&mut self) -> Result<(), H2Error> {
        self.pending.retain(|cmd| !cmd.is_cancelled());
        let to_cancel: Vec<(u32, bool)> = self
            .streams
            .iter()
            .filter(|(_, actor)| {
                actor
                    .response_tx
                    .as_ref()
                    .is_some_and(ResponseSink::is_cancelled)
            })
            .map(|(id, actor)| (id, !matches!(actor.response, ResponseState::Initial(_))))
            .collect();
        if to_cancel.iter().any(|&(_, answered)| !answered) {
            self.unanswered_timeout.store(true, Ordering::Release);
        }

        for (sid, _) in to_cancel {
            tracing::debug!(
                target: "leyline::h2",
                stream_id = sid,
                "caller cancelled — RST_STREAM(CANCEL) to release the slot"
            );
            let err = H2Error::Stream {
                stream_id: sid,
                code: ErrorCode::Cancel,
            };
            if self
                .streams
                .get(sid)
                .is_some_and(|actor| actor.state.is_closed())
            {
                self.fail_stream(sid, err);
            } else {
                self.reset_stream(sid, err, ErrorCode::Cancel)?;
            }
        }
        Ok(())
    }

    fn check_ping_deadlines(&self) -> Result<(), H2Error> {
        let now = std::time::Instant::now();
        if self.pings.iter().any(|&(_, _, deadline)| deadline <= now) {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "the peer did not acknowledge a liveness PING in time".into(),
            });
        }
        Ok(())
    }

    pub(super) fn admit(&self) -> Admit {
        if self.peer_goaway_last_stream.is_some()
            || self.output.stalled()
            || self.next_stream_id > U31_MAX
        {
            return Admit::Refuse(H2Error::NotSent);
        }
        if let Some(limit) = self.peer_settings.max_concurrent_streams
            && self.streams.len() >= limit as usize
            && self.active_stream_count() >= limit
        {
            return Admit::Defer;
        }
        Admit::Open
    }

    pub(super) fn alloc_stream_id(&mut self) -> u32 {
        let stream_id = self.next_stream_id;
        self.next_stream_id = stream_id + 2;
        if self.next_stream_id > U31_MAX {
            self.closed.store(true, Ordering::Release);
        }
        stream_id
    }

    pub(super) fn effective_send_window(&self, stream_id: u32) -> usize {
        let conn = self.conn_send_window.max(0) as usize;
        let stream = self
            .streams
            .get(stream_id)
            .map(|i| i.send_window.max(0) as usize)
            .unwrap_or(0);
        conn.min(stream)
    }

    pub(super) fn finish_remote(&mut self, stream_id: u32) -> Result<(), H2Error> {
        if self
            .streams
            .get(stream_id)
            .is_some_and(|actor| actor.response.finish().is_err() || actor.length_mismatch())
        {
            return self.reset_protocol(stream_id);
        }
        let local_open = self
            .streams
            .get(stream_id)
            .is_some_and(|a| !a.state.is_closed());
        if local_open {
            RstStreamFrame {
                stream_id,
                error_code: ErrorCode::NoError,
            }
            .encode(self.writer.out());
            if let Some(actor) = self.streams.get_mut(stream_id) {
                actor
                    .state
                    .transition(StreamEvent::SendRstStream(ErrorCode::NoError))
                    .map_err(|error| map_state_err(stream_id, error))?;
                actor.cancel_upload();
            }
            self.streams.unpark(stream_id);
        }
        if let Some(actor) = self.streams.get_mut(stream_id)
            && !actor.stalled.is_empty()
        {
            actor.remote_done = true;
            return Ok(());
        }
        self.complete_stream(stream_id);
        Ok(())
    }

    pub(super) fn flush_stalled(&mut self) {
        for (sid, done) in self.streams.flush_stalled() {
            if done {
                self.complete_stream(sid);
            } else {
                self.maybe_top_up_stream_window(sid);
            }
        }
    }

    pub(super) fn complete_stream(&mut self, stream_id: u32) {
        if let Some(mut actor) = self.streams.remove(stream_id) {
            actor.deliver_ok();
        }
    }

    pub(super) fn reset_stream(
        &mut self,
        stream_id: u32,
        err: H2Error,
        code: ErrorCode,
    ) -> Result<(), H2Error> {
        if let Some(actor) = self.streams.get_mut(stream_id) {
            actor
                .state
                .transition(StreamEvent::SendRstStream(code))
                .map_err(|error| map_state_err(stream_id, error))?;
        }
        RstStreamFrame {
            stream_id,
            error_code: code,
        }
        .encode(self.writer.out());
        self.fail_stream(stream_id, err);
        Ok(())
    }

    pub(super) fn reset_protocol(&mut self, stream_id: u32) -> Result<(), H2Error> {
        self.reset_stream(
            stream_id,
            H2Error::Stream {
                stream_id,
                code: ErrorCode::ProtocolError,
            },
            ErrorCode::ProtocolError,
        )
    }

    pub(super) fn fail_stream(&mut self, stream_id: u32, err: H2Error) {
        if let Some(mut actor) = self.streams.remove(stream_id) {
            actor.deliver_err(err);
        }
    }

    pub(super) fn active_stream_count(&self) -> u32 {
        self.streams
            .values()
            .filter(|a| {
                matches!(
                    a.state,
                    StreamState::Open
                        | StreamState::HalfClosedLocal
                        | StreamState::HalfClosedRemote
                )
            })
            .count() as u32
    }
}
