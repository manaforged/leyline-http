use std::sync::atomic::Ordering;

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
            .get(stream_id)
            .map(|a| a.stalled.is_empty() && a.recv_window < initial / 2)
            .unwrap_or(false);
        if needs_update {
            let current = self
                .streams
                .get(stream_id)
                .map(|a| a.recv_window)
                .unwrap_or(0);
            let increment = (initial - current).clamp(1, 0x7FFF_FFFF) as u32;
            self.writer
                .write_window_update(&WindowUpdateFrame {
                    stream_id,
                    increment,
                })
                .await?;
            if let Some(actor) = self.streams.get_mut(stream_id) {
                actor.recv_window += increment as i64;
            }
        }
        Ok(())
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
            if actor.remote_done {
                actor.deliver_ok();
            } else {
                actor.deliver_err(clone_err(&final_err));
            }
        }
        while let Some(cmd) = self.pending.pop_front() {
            match cmd {
                DriverCommand::SendRequest { response_tx, .. } => {
                    let _ = response_tx.send(Err(clone_err(&final_err)));
                }
                DriverCommand::SendRequestEx { sink, .. }
                | DriverCommand::OpenConnect { sink, .. } => {
                    send_err_to_sink(sink, clone_err(&final_err));
                }
                DriverCommand::Ping { .. } => {}
            }
        }
        while let Ok(cmd) = self.command_rx.try_recv() {
            match cmd {
                DriverCommand::SendRequest { response_tx, .. } => {
                    let _ = response_tx.send(Err(clone_err(&final_err)));
                }
                DriverCommand::SendRequestEx { sink, .. }
                | DriverCommand::OpenConnect { sink, .. } => {
                    send_err_to_sink(sink, clone_err(&final_err));
                }
                DriverCommand::Ping { .. } => {}
            }
        }
        result
    }

    pub(super) async fn event_loop(&mut self) -> Result<(), H2Error> {
        let mut sweep_tick = tokio::time::interval(std::time::Duration::from_millis(100));
        sweep_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut flush_tick = tokio::time::interval(std::time::Duration::from_millis(1));
        flush_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if self.shutdown_started && self.streams.is_empty() {
                let _ = self.writer.write_goaway(0, ErrorCode::NoError).await;
                let _ = self.writer.flush().await;
                return Ok(());
            }
            let mut tick_fired = false;
            for _ in 0..128 {
                if !self.reader.buffered() {
                    break;
                }
                match self.reader.next().await? {
                    Some(f) => {
                        self.on_inbound_frame(f).await?;
                        if !self.pending.is_empty() {
                            self.drain_pending().await?;
                        }
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
            for _ in 0..self.command_rx.len() {
                let Ok(next) = self.command_rx.try_recv() else {
                    break;
                };
                self.on_command(next).await?;
            }
            for _ in 0..self.body_chunk_rx.len() {
                let Ok(chunk) = self.body_chunk_rx.try_recv() else {
                    break;
                };
                self.on_body_chunk(chunk).await?;
            }
            if !self.pending.is_empty() {
                self.drain_pending().await?;
            }
            self.try_drain_pending().await?;
            if self.writer.pending() > 0 && !self.reader.buffered() {
                self.writer.flush().await?;
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
                maybe_cmd = self.command_rx.recv(), if !self.shutdown_started => {
                    match maybe_cmd {
                        Some(cmd) => {
                            self.on_command(cmd).await?;
                        }
                        None => self.shutdown_started = true,
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
                _ = flush_tick.tick(), if self.has_stalled() => {
                    self.flush_stalled().await?;
                }
            }

            for _ in 0..self.command_rx.len() {
                let Ok(next) = self.command_rx.try_recv() else {
                    break;
                };
                self.on_command(next).await?;
            }
            self.try_drain_pending().await?;

            if tick_fired {
                self.sweep_cancelled_streams().await?;
            }
        }
    }

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
                        ..
                    }) => {
                        let headers_dead =
                            headers_tx.as_ref().map(|t| t.is_closed()).unwrap_or(true);
                        let body_dead = body_tx.is_closed();
                        headers_dead && body_dead
                    }
                    None => false,
                }
            })
            .map(|(id, _)| id)
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

    pub(super) fn reject_after_goaway(&self) -> Result<(), H2Error> {
        if self.peer_goaway_last_stream.is_some() {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "peer sent GOAWAY, refusing new streams".into(),
            });
        }
        Ok(())
    }

    pub(super) fn peer_ready(&self) -> bool {
        self.peer_greeted
    }

    pub(super) fn check_stream_capacity(&self) -> Result<(), H2Error> {
        if let Some(limit) = self.peer_settings.max_concurrent_streams
            && self.streams.len() >= limit as usize
            && self.active_stream_count() >= limit
        {
            return Err(H2Error::Connection {
                code: ErrorCode::RefusedStream,
                reason: "MAX_CONCURRENT_STREAMS exceeded".into(),
            });
        }
        if self.next_stream_id > 0x7FFF_FFFF {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "stream ID space exhausted".into(),
            });
        }
        Ok(())
    }

    pub(super) fn admit_new_stream(&self) -> Result<(), H2Error> {
        self.reject_after_goaway()?;
        self.check_stream_capacity()
    }

    pub(super) fn alloc_stream_id(&mut self) -> u32 {
        let stream_id = self.next_stream_id;
        self.next_stream_id = stream_id + 2;
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

    pub(super) fn park_stream(&mut self, stream_id: u32, pending: PendingSend) {
        if let Some(actor) = self.streams.get_mut(stream_id) {
            actor.pending_send = Some(pending);
            if !self.buffered_pending.contains(&stream_id) {
                self.buffered_pending.push_back(stream_id);
            }
        }
    }

    pub(super) async fn finish_remote(&mut self, stream_id: u32) -> Result<(), H2Error> {
        let local_open = self
            .streams
            .get(stream_id)
            .is_some_and(|a| !a.state.is_closed());
        if local_open {
            let _ = self
                .writer
                .write_rst_stream(stream_id, ErrorCode::NoError)
                .await;
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

    pub(super) fn has_stalled(&self) -> bool {
        self.stalled > 0
    }

    pub(super) async fn flush_stalled(&mut self) -> Result<(), H2Error> {
        let mut drained = Vec::new();
        for (sid, actor) in self.streams.iter_mut() {
            let Some(ResponseSink::StreamingEx { body_tx, .. }) = actor.response_tx.as_ref() else {
                continue;
            };
            while let Some(chunk) = actor.stalled.pop_front() {
                self.stalled -= 1;
                match body_tx.try_send(Ok(chunk)) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(Ok(chunk))) => {
                        actor.stalled.push_front(chunk);
                        self.stalled += 1;
                        break;
                    }
                    Err(_) => break,
                }
            }
            if actor.stalled.is_empty() {
                drained.push((sid, actor.remote_done));
            }
        }
        for (sid, done) in drained {
            if done {
                self.complete_stream(sid);
            } else {
                self.maybe_top_up_stream_window(sid).await?;
            }
        }
        Ok(())
    }

    pub(super) fn complete_stream(&mut self, stream_id: u32) {
        if let Some(mut actor) = self.streams.remove(stream_id) {
            self.stalled = self.stalled.saturating_sub(actor.stalled.len());
            actor.deliver_ok();
        }
        self.buffered_pending.retain(|&s| s != stream_id);
    }

    pub(super) fn fail_stream(&mut self, stream_id: u32, err: H2Error) {
        if let Some(mut actor) = self.streams.remove(stream_id) {
            self.stalled = self.stalled.saturating_sub(actor.stalled.len());
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
