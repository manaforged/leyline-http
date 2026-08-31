//! Driver lifecycle: run loop, stream admission, stream completion.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite};

use crate::h2::error::{ErrorCode, H2Error};
use crate::h2::stream_state::StreamState;

use super::*;

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) async fn maybe_top_up_conn_window(&mut self) -> Result<(), H2Error> {
        let initial = self.config.initial_connection_window_size as i64;
        if self.conn_recv_window < initial / 2 {
            let increment = (initial - self.conn_recv_window).clamp(1, 0x7FFF_FFFF) as u32;
            self.writer
                .write_window_update(&WindowUpdateFrame {
                    stream_id: 0,
                    increment,
                })
                .await?;
            self.conn_recv_window += increment as i64;
        }
        Ok(())
    }

    pub(super) async fn maybe_top_up_stream_window(
        &mut self,
        stream_id: u32,
    ) -> Result<(), H2Error> {
        let initial = self.config.advertised_initial_window_size() as i64;
        let needs_update = self
            .streams
            .get(&stream_id)
            .map(|a| a.recv_window < initial / 2)
            .unwrap_or(false);
        if needs_update {
            let current = self
                .streams
                .get(&stream_id)
                .map(|a| a.recv_window)
                .unwrap_or(0);
            let increment = (initial - current).clamp(1, 0x7FFF_FFFF) as u32;
            self.writer
                .write_window_update(&WindowUpdateFrame {
                    stream_id,
                    increment,
                })
                .await?;
            if let Some(actor) = self.streams.get_mut(&stream_id) {
                actor.recv_window += increment as i64;
            }
        }
        Ok(())
    }

    pub(super) async fn graceful_shutdown(&mut self) -> Result<(), H2Error> {
        self.shutdown_started = true;
        let last = self.next_stream_id.saturating_sub(2);
        let _ = self.writer.write_goaway(last, ErrorCode::NoError).await;
        let _ = self.writer.flush().await;

        if self.streams.is_empty() {
            return Ok(());
        }

        let mut body_rx_open = true;
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            if self.streams.is_empty() {
                return Ok(());
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(());
            }
            let timeout = deadline - now;
            tokio::select! {
                biased;
                frame = self.reader.next() => {
                    match frame {
                        Ok(Some(f)) => self.on_inbound_frame(f).await?,
                        Ok(None) => return Ok(()),
                        Err(e) => return Err(e),
                    }
                    self.try_drain_pending().await?;
                }
                maybe_chunk = self.body_chunk_rx.recv(), if body_rx_open => {
                    match maybe_chunk {
                        Some(c) => {
                            self.on_body_chunk(c).await?;
                            self.try_drain_pending().await?;
                        }
                        None => body_rx_open = false,
                    }
                }
                _ = tokio::time::sleep(timeout) => {
                    return Ok(());
                }
            }
        }
    }
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
            Err(e) => clone_err(e),
        };
        for (_, mut actor) in self.streams.drain() {
            actor.deliver_err(clone_err(&final_err));
        }
        while let Some(cmd) = self.pending.pop_front() {
            match cmd {
                DriverCommand::SendRequest { response_tx, .. } => {
                    let _ = response_tx.send(Err(clone_err(&final_err)));
                }
                DriverCommand::SendRequestEx { response_tx, .. } => {
                    let _ = response_tx.send(Err(clone_err(&final_err)));
                }
                DriverCommand::OpenConnect { headers_tx, .. } => {
                    let _ = headers_tx.send(Err(clone_err(&final_err)));
                }
            }
        }
        while let Ok(cmd) = self.command_rx.try_recv() {
            match cmd {
                DriverCommand::SendRequest { response_tx, .. } => {
                    let _ = response_tx.send(Err(clone_err(&final_err)));
                }
                DriverCommand::SendRequestEx { response_tx, .. } => {
                    let _ = response_tx.send(Err(clone_err(&final_err)));
                }
                DriverCommand::OpenConnect { headers_tx, .. } => {
                    let _ = headers_tx.send(Err(clone_err(&final_err)));
                }
            }
        }
        result
    }

    pub(super) async fn event_loop(&mut self) -> Result<(), H2Error> {
        let mut sweep_tick = tokio::time::interval(std::time::Duration::from_millis(100));
        sweep_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let mut tick_fired = false;
            if !self.pending.is_empty() {
                self.drain_pending().await?;
            }
            tokio::select! {
                biased;
                frame = self.reader.next() => {
                    match frame? {
                        Some(f) => {
                            self.on_inbound_frame(f).await?;
                            self.drain_pending().await?;
                        }
                        None => {
                            return if self.shutdown_started {
                                Ok(())
                            } else {
                                Err(H2Error::Connection {
                                    code: ErrorCode::NoError,
                                    reason: "peer closed connection".into(),
                                })
                            };
                        }
                    }
                }
                maybe_cmd = self.command_rx.recv() => {
                    match maybe_cmd {
                        Some(cmd) => {
                            self.on_command(cmd).await?;
                            while let Ok(next) = self.command_rx.try_recv() {
                                self.on_command(next).await?;
                            }
                        }
                        None => {
                            return self.graceful_shutdown().await;
                        }
                    }
                }
                maybe_chunk = self.body_chunk_rx.recv() => {
                    if let Some(c) = maybe_chunk {
                        self.on_body_chunk(c).await?;
                    }
                }
                _ = sweep_tick.tick() => {
                    tick_fired = true;
                }
            }

            self.try_drain_pending().await?;

            if tick_fired {
                self.sweep_cancelled_streams().await?;
            }
        }
    }

    /// Walk the streams table; for any entry whose caller has dropped the response oneshot (or whose streaming body channel is closed on the reader side), send RST_STREAM(CANCEL) and clean up the actor.
    pub(super) async fn sweep_cancelled_streams(&mut self) -> Result<(), H2Error> {
        let to_cancel: Vec<u32> = self
            .streams
            .iter()
            .filter(|(_, actor)| {
                if actor.state.is_closed() {
                    return false;
                }
                if matches!(
                    actor.send_body_input,
                    SendBodyInput::Streaming { closed: false, .. }
                ) {
                    return false;
                }
                match actor.response_tx.as_ref() {
                    Some(ResponseSink::Buffered(tx)) => tx.is_closed(),
                    Some(ResponseSink::BufferedEx(tx)) => tx.is_closed(),
                    Some(ResponseSink::StreamingEx {
                        headers_tx,
                        body_tx,
                    }) => {
                        let headers_dead =
                            headers_tx.as_ref().map(|t| t.is_closed()).unwrap_or(true);
                        let body_dead = body_tx.is_closed();
                        headers_dead && body_dead
                    }
                    None => false,
                }
            })
            .map(|(&id, _)| id)
            .collect();

        for sid in to_cancel {
            tracing::debug!(
                target: "leyline::h2",
                stream_id = sid,
                "caller cancelled — RST_STREAM(CANCEL) to release the slot"
            );
            let _ = self.writer.write_rst_stream(sid, ErrorCode::Cancel).await;
            let err = H2Error::Stream {
                stream_id: sid,
                code: ErrorCode::Cancel,
            };
            self.fail_stream(sid, err);
        }
        Ok(())
    }

    /// Reject a new stream if the peer has sent GOAWAY.
    pub(super) fn reject_after_goaway(&self) -> Result<(), H2Error> {
        if self.peer_goaway_last_stream.is_some() {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "peer sent GOAWAY, refusing new streams".into(),
            });
        }
        Ok(())
    }

    /// Peer SETTINGS received — the concurrent-stream limit is known and admission decisions are meaningful.
    pub(super) fn peer_ready(&self) -> bool {
        self.peer_greeted
    }

    /// Reject a new stream if the peer's MAX_CONCURRENT_STREAMS is reached or our client stream-ID space is exhausted.
    pub(super) fn check_stream_capacity(&self) -> Result<(), H2Error> {
        if let Some(limit) = self.peer_settings.max_concurrent_streams {
            if self.active_stream_count() >= limit {
                return Err(H2Error::Connection {
                    code: ErrorCode::RefusedStream,
                    reason: "MAX_CONCURRENT_STREAMS exceeded".into(),
                });
            }
        }
        if self.next_stream_id > 0x7FFF_FFFF {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "stream ID space exhausted".into(),
            });
        }
        Ok(())
    }

    /// Full admission check for a plain new stream: GOAWAY refusal then capacity.
    pub(super) fn admit_new_stream(&self) -> Result<(), H2Error> {
        self.reject_after_goaway()?;
        self.check_stream_capacity()
    }

    /// Allocate the next client stream ID (odd, +2) and advance the counter.
    pub(super) fn alloc_stream_id(&mut self) -> u32 {
        let stream_id = self.next_stream_id;
        self.next_stream_id = stream_id + 2;
        stream_id
    }

    /// Bytes we may send on `stream_id` right now: the smaller of the connection- and stream-level send windows, each clamped to >= 0.
    pub(super) fn effective_send_window(&self, stream_id: u32) -> usize {
        let conn = self.conn_send_window.max(0) as usize;
        let stream = self
            .streams
            .get(&stream_id)
            .map(|i| i.send_window.max(0) as usize)
            .unwrap_or(0);
        conn.min(stream)
    }

    pub(super) fn park_stream(&mut self, stream_id: u32, pending: PendingSend) {
        if let Some(actor) = self.streams.get_mut(&stream_id) {
            actor.pending_send = Some(pending);
            if !self.buffered_pending.contains(&stream_id) {
                self.buffered_pending.push_back(stream_id);
            }
        }
    }

    pub(super) fn complete_stream(&mut self, stream_id: u32) {
        if let Some(mut actor) = self.streams.remove(&stream_id) {
            actor.deliver_ok();
        }
        self.buffered_pending.retain(|&s| s != stream_id);
    }

    pub(super) fn fail_stream(&mut self, stream_id: u32, err: H2Error) {
        if let Some(mut actor) = self.streams.remove(&stream_id) {
            actor.deliver_err(err);
        }
        self.buffered_pending.retain(|&s| s != stream_id);
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
