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
        // Must match the seed in `StreamActor::new` call sites: the
        // stream's receive window is OUR advertised
        // SETTINGS_INITIAL_WINDOW_SIZE. `peer_settings.initial_window_size`
        // governs the send direction and made this threshold
        // unreachable against small-window peers (stall after the
        // advertised window on any larger body).
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
        // Send GOAWAY(NO_ERROR, last_stream_id = next_stream_id - 2).
        let last = self.next_stream_id.saturating_sub(2);
        let _ = self.writer.write_goaway(last, ErrorCode::NoError).await;
        let _ = self.writer.flush().await;

        if self.streams.is_empty() {
            return Ok(());
        }

        // Drain — wait for in-flight responses up to a small timeout.
        // In-flight streaming UPLOADS must keep pumping here too:
        // omitting the body_chunk_rx arm starved them, the server
        // never saw END_STREAM, and the timeout surfaced a spurious
        // error on a healthy transfer.
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
                        // All senders dropped — disarm the arm or a
                        // closed channel busy-loops the select until
                        // the deadline.
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
        // Mark closed so handles stop enqueuing new commands.
        self.closed.store(true, Ordering::Release);
        // Fail any remaining pending requests with the final status.
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
        // Drain remaining commands in the channel and fail them.
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
        // Periodic tick so the cancel-safety sweep fires even when the
        // connection is otherwise idle. 100 ms is fine-grained enough
        // that a cancelled `send_request` can't peg a connection on
        // `MAX_CONCURRENT_STREAMS` for noticeable human time, and
        // coarse enough that it costs ~10 wakeups/second of idle CPU
        // — negligible compared to a real request flow.
        let mut sweep_tick = tokio::time::interval(std::time::Duration::from_millis(100));
        sweep_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            // Only the periodic tick triggers the O(streams) cancel sweep —
            // running it after every frame/command/chunk made cleanup
            // O(event-rate × streams) on the single driver. A dropped caller's
            // slot is reclaimed within one tick (100 ms), which is the bounded
            // cadence the sweep was designed for.
            let mut tick_fired = false;
            tokio::select! {
                biased;
                frame = self.reader.next() => {
                    match frame? {
                        Some(f) => self.on_inbound_frame(f).await?,
                        None => {
                            // Reader EOF. If we've initiated shutdown, this is ok.
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
                        Some(cmd) => self.on_command(cmd).await?,
                        None => {
                            // Last handle dropped — graceful shutdown.
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

            // After any event we try to drain pending sends because the
            // write window may have grown (WINDOW_UPDATE / SETTINGS).
            self.try_drain_pending().await?;

            // Cancel-safety sweep: if the caller dropped the response
            // oneshot (e.g. `tokio::select!` lost this branch, or an
            // outer timeout fired), we must proactively RST_STREAM
            // the orphaned stream — otherwise its `StreamActor`
            // lingers, consuming a MAX_CONCURRENT_STREAMS slot and
            // flow-control window until the server closes from its
            // side. Under aggressive cancellation this pegs the
            // connection at the concurrent-stream limit. Gated to the tick so
            // it costs O(streams) at ~10 Hz, not O(streams) per event.
            if tick_fired {
                self.sweep_cancelled_streams().await?;
            }
        }
    }

    /// Walk the streams table; for any entry whose caller has dropped
    /// the response oneshot (or whose streaming body channel is
    /// closed on the reader side), send RST_STREAM(CANCEL) and clean
    /// up the actor.
    pub(super) async fn sweep_cancelled_streams(&mut self) -> Result<(), H2Error> {
        // Collect first to avoid mutating `self.streams` under the
        // borrow of the iteration. Streams in Closed state are
        // already cleaned up elsewhere. Streams with an active
        // streaming-body relay (extended CONNECT, streaming upload)
        // are skipped: those use the write relay's EOF signal to
        // emit a graceful END_STREAM, so an eager RST would race the
        // natural close.
        let to_cancel: Vec<u32> = self
            .streams
            .iter()
            .filter(|(_, actor)| {
                if actor.state.is_closed() {
                    return false;
                }
                // Active streaming-body upload or CONNECT stream —
                // graceful close flows through the relay, not RST.
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
                        // The caller cancelled if BOTH the headers
                        // oneshot AND the body channel are gone.
                        // Either alone might be dropped legitimately
                        // by the driver mid-flight (e.g. after headers
                        // have already been delivered).
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

    // -------------------------------------------------------------------
    // Command handling
    // -------------------------------------------------------------------

    /// Reject a new stream if the peer has sent GOAWAY. Checked first at
    /// every stream-open site so a going-away connection always reports the
    /// same error before any capacity check.
    pub(super) fn reject_after_goaway(&self) -> Result<(), H2Error> {
        if self.peer_goaway_last_stream.is_some() {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "peer sent GOAWAY, refusing new streams".into(),
            });
        }
        Ok(())
    }

    /// Reject a new stream if the peer's MAX_CONCURRENT_STREAMS is reached
    /// or our client stream-ID space is exhausted.
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

    /// Full admission check for a plain new stream: GOAWAY refusal then
    /// capacity. Extended CONNECT composes the two halves itself so its
    /// ENABLE_CONNECT_PROTOCOL check keeps its original position between
    /// the GOAWAY and capacity checks.
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

    /// Bytes we may send on `stream_id` right now: the smaller of the
    /// connection- and stream-level send windows, each clamped to >= 0. A
    /// missing stream yields 0.
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
        // Clean up pending queue.
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
