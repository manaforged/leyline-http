use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::h2::error::{ErrorCode, H2Error};
use crate::h2::stream_state::StreamEvent;

use super::*;

mod headers;

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) async fn on_body_chunk(&mut self, chunk: BodyChunkIn) -> Result<(), H2Error> {
        match chunk {
            BodyChunkIn::Chunk { stream_id, data } => {
                let should_try_write = if let Some(actor) = self.streams.get_mut(stream_id) {
                    if let SendBodyInput::Streaming { pending_buf, .. } = &mut actor.send_body_input
                    {
                        pending_buf.push_back(data);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if should_try_write {
                    self.try_pump_streaming_body(stream_id).await?;
                }
            }
            BodyChunkIn::Eof { stream_id, error } => {
                let should_try_write = if let Some(actor) = self.streams.get_mut(stream_id) {
                    if let SendBodyInput::Streaming {
                        closed, error: e, ..
                    } = &mut actor.send_body_input
                    {
                        *closed = true;
                        if let Some(err) = error {
                            *e = Some(err);
                        }
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if should_try_write {
                    self.try_pump_streaming_body(stream_id).await?;
                }
            }
        }
        Ok(())
    }

    pub(super) async fn try_pump_streaming_body(&mut self, stream_id: u32) -> Result<(), H2Error> {
        loop {
            let parked = self
                .streams
                .get(stream_id)
                .is_some_and(|a| a.pending_send.is_some());
            if parked {
                return Ok(());
            }
            let next_chunk: Option<Bytes> =
                self.streams
                    .get_mut(stream_id)
                    .and_then(|a| match &mut a.send_body_input {
                        SendBodyInput::Streaming { pending_buf, .. } => pending_buf.pop_front(),
                        _ => None,
                    });

            let closed = self
                .streams
                .get(stream_id)
                .is_some_and(|a| match &a.send_body_input {
                    SendBodyInput::Streaming { closed, .. } => *closed,
                    SendBodyInput::None => false,
                });

            let already_closed = self
                .streams
                .get(stream_id)
                .map(|a| a.send_closed)
                .unwrap_or(true);
            if already_closed {
                return Ok(());
            }

            if next_chunk.is_none()
                && let Some(cause) = self.take_body_error(stream_id)
            {
                let _ = self
                    .writer
                    .write_rst_stream(stream_id, ErrorCode::InternalError)
                    .await;
                self.fail_stream(stream_id, H2Error::RequestBody(cause));
                return Ok(());
            }

            match next_chunk {
                Some(chunk) if !chunk.is_empty() => {
                    self.write_streaming_chunk(stream_id, chunk, closed).await?;
                }
                _ => {
                    if closed {
                        self.end_send_side(stream_id).await?;
                    }
                    return Ok(());
                }
            }
        }
    }

    async fn end_send_side(&mut self, stream_id: u32) -> Result<(), H2Error> {
        if let Some(actor) = self.streams.get_mut(stream_id) {
            actor
                .state
                .transition(StreamEvent::SendData { end_stream: true })
                .map_err(|e| map_state_err(stream_id, e))?;
            actor.send_closed = true;
        }
        self.writer
            .write_data(&DataFrame {
                stream_id,
                end_stream: true,
                data: Bytes::new(),
                wire_len: 0,
            })
            .await?;
        self.writer.flush().await?;
        Ok(())
    }

    fn take_body_error(&mut self, stream_id: u32) -> Option<std::io::Error> {
        match &mut self.streams.get_mut(stream_id)?.send_body_input {
            SendBodyInput::Streaming { error, .. } => error.take(),
            SendBodyInput::None => None,
        }
    }

    pub(super) async fn write_streaming_chunk(
        &mut self,
        stream_id: u32,
        mut chunk: Bytes,
        producer_closed: bool,
    ) -> Result<(), H2Error> {
        while !chunk.is_empty() {
            let window = self.effective_send_window(stream_id);
            if window == 0 {
                self.park_stream(stream_id, PendingSend { remaining: chunk });
                return Ok(());
            }
            let max_frame = self.peer_settings.max_frame_size as usize;
            let chunk_size = chunk.len().min(max_frame).min(window);
            let piece = chunk.slice(0..chunk_size);
            chunk = chunk.slice(chunk_size..);

            let is_last = chunk.is_empty() && producer_closed && self.upload_drained(stream_id);

            if let Some(actor) = self.streams.get_mut(stream_id) {
                if let Err(e) = actor.state.transition(StreamEvent::SendData {
                    end_stream: is_last,
                }) {
                    return Err(map_state_err(stream_id, e));
                }
                if is_last {
                    actor.send_closed = true;
                }
            }

            let wire_len = piece.len() as u64;
            self.writer
                .write_data(&DataFrame {
                    stream_id,
                    end_stream: is_last,
                    data: piece,
                    wire_len,
                })
                .await?;

            self.note_upload_sent(stream_id, chunk_size);
            if is_last {
                self.writer.flush().await?;
            }
        }
        Ok(())
    }

    fn upload_drained(&self, stream_id: u32) -> bool {
        self.streams
            .get(stream_id)
            .is_none_or(|a| match &a.send_body_input {
                SendBodyInput::Streaming { pending_buf, .. } => pending_buf.is_empty(),
                SendBodyInput::None => true,
            })
    }

    fn note_upload_sent(&mut self, stream_id: u32, sent: usize) {
        self.conn_send_window -= sent as i64;
        if let Some(actor) = self.streams.get_mut(stream_id) {
            actor.send_window -= sent as i64;
            if let Some(credit) = &actor.upload_credit {
                credit.add_permits(sent);
            }
        }
    }

    pub(super) async fn write_body_or_park(
        &mut self,
        stream_id: u32,
        body: Bytes,
    ) -> Result<(), H2Error> {
        let mut remaining = body;
        while !remaining.is_empty() {
            let window = self.effective_send_window(stream_id);
            if window == 0 {
                self.park_stream(stream_id, PendingSend { remaining });
                return Ok(());
            }
            let max_frame = self.peer_settings.max_frame_size as usize;
            let chunk_size = remaining.len().min(max_frame).min(window);
            let chunk = remaining.slice(0..chunk_size);
            remaining = remaining.slice(chunk_size..);
            let data_end_stream = remaining.is_empty();

            if let Some(actor) = self.streams.get_mut(stream_id)
                && let Err(e) = actor.state.transition(StreamEvent::SendData {
                    end_stream: data_end_stream,
                })
            {
                return Err(map_state_err(stream_id, e));
            }

            let wire_len = chunk.len() as u64;
            self.writer
                .write_data(&DataFrame {
                    stream_id,
                    end_stream: data_end_stream,
                    data: chunk,
                    wire_len,
                })
                .await?;

            self.conn_send_window -= chunk_size as i64;
            if let Some(actor) = self.streams.get_mut(stream_id) {
                actor.send_window -= chunk_size as i64;
            }
        }

        Ok(())
    }

    pub(super) async fn try_drain_pending(&mut self) -> Result<(), H2Error> {
        if self.buffered_pending.is_empty() {
            return Ok(());
        }
        let mut progress_count = self.buffered_pending.len();
        while progress_count > 0 && !self.buffered_pending.is_empty() {
            progress_count -= 1;
            let sid = match self.buffered_pending.pop_front() {
                Some(s) => s,
                None => break,
            };
            let pending = match self
                .streams
                .get_mut(sid)
                .and_then(|a| a.pending_send.take())
            {
                Some(p) => p,
                None => continue,
            };

            let is_streaming = self
                .streams
                .get(sid)
                .map(|a| matches!(a.send_body_input, SendBodyInput::Streaming { .. }))
                .unwrap_or(false);
            if is_streaming {
                if let Some(actor) = self.streams.get_mut(sid)
                    && let SendBodyInput::Streaming { pending_buf, .. } = &mut actor.send_body_input
                    && !pending.remaining.is_empty()
                {
                    pending_buf.push_front(pending.remaining);
                }
                if let Err(e) = self.try_pump_streaming_body(sid).await {
                    self.fail_stream(sid, e);
                } else {
                    self.writer.flush().await?;
                }
                continue;
            }

            let result = self.write_body_or_park(sid, pending.remaining).await;
            match result {
                Ok(()) => {
                    self.writer.flush().await?;
                }
                Err(e) => {
                    self.fail_stream(sid, e);
                }
            }
        }
        Ok(())
    }
}
