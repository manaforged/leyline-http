use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::h2::error::ErrorCode;

use crate::h2::H2Error;
use crate::h2::stream_state::StreamEvent;

use super::*;

mod headers;

#[derive(Clone, Copy)]
pub(super) enum DataEnd {
    Buffered,
    Streaming { producer_closed: bool },
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) fn on_body_chunk(&mut self, chunk: BodyChunkIn) -> Result<(), H2Error> {
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
                    self.try_pump_streaming_body(stream_id)?;
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
                    self.try_pump_streaming_body(stream_id)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn try_pump_streaming_body(&mut self, stream_id: u32) -> Result<(), H2Error> {
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
                .map(|a| !matches!(a.state, StreamState::Open | StreamState::HalfClosedRemote))
                .unwrap_or(true);
            if already_closed {
                return Ok(());
            }

            if next_chunk.is_none()
                && let Some(cause) = self.take_body_error(stream_id)
            {
                RstStreamFrame {
                    stream_id,
                    error_code: ErrorCode::InternalError,
                }
                .encode(self.writer.out());
                self.fail_stream(stream_id, H2Error::RequestBody(cause));
                return Ok(());
            }

            match next_chunk {
                Some(chunk) if !chunk.is_empty() => {
                    self.write_data_frames(
                        stream_id,
                        chunk,
                        DataEnd::Streaming {
                            producer_closed: closed,
                        },
                    )?;
                }
                _ => {
                    if closed {
                        self.end_send_side(stream_id)?;
                    }
                    return Ok(());
                }
            }
        }
    }

    fn end_send_side(&mut self, stream_id: u32) -> Result<(), H2Error> {
        if let Some(actor) = self.streams.get_mut(stream_id) {
            actor
                .state
                .transition(StreamEvent::SendData { end_stream: true })
                .map_err(|e| map_state_err(stream_id, e))?;
        }
        DataFrame {
            stream_id,
            end_stream: true,
            data: Bytes::new(),
            wire_len: 0,
        }
        .encode(self.writer.out());
        Ok(())
    }

    fn take_body_error(&mut self, stream_id: u32) -> Option<std::io::Error> {
        match &mut self.streams.get_mut(stream_id)?.send_body_input {
            SendBodyInput::Streaming { error, .. } => error.take(),
            SendBodyInput::None => None,
        }
    }

    pub(super) fn write_data_frames(
        &mut self,
        stream_id: u32,
        mut remaining: Bytes,
        end: DataEnd,
    ) -> Result<(), H2Error> {
        while !remaining.is_empty() {
            let window = self.effective_send_window(stream_id);
            if window == 0 || self.writer.congested() {
                self.streams.park(stream_id, PendingSend { remaining });
                return Ok(());
            }
            let max_frame = self.peer_settings.max_frame_size as usize;
            let chunk_size = remaining
                .len()
                .min(max_frame)
                .min(window)
                .min(self.writer.allowance());
            let piece = remaining.slice(0..chunk_size);
            remaining = remaining.slice(chunk_size..);

            let end_stream = match end {
                DataEnd::Buffered => remaining.is_empty(),
                DataEnd::Streaming { producer_closed } => {
                    remaining.is_empty() && producer_closed && self.upload_drained(stream_id)
                }
            };

            if let Some(actor) = self.streams.get_mut(stream_id)
                && let Err(e) = actor.state.transition(StreamEvent::SendData { end_stream })
            {
                return Err(map_state_err(stream_id, e));
            }

            let wire_len = piece.len() as u64;
            DataFrame {
                stream_id,
                end_stream,
                data: piece,
                wire_len,
            }
            .encode(self.writer.out());

            self.note_upload_sent(stream_id, chunk_size);
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
            if let Some(upload) = &actor.upload {
                upload.sent(sent);
            }
        }
    }

    pub(super) fn try_drain_pending(&mut self) {
        let mut progress_count = self.streams.parked();
        while progress_count > 0 && self.streams.parked() > 0 && !self.writer.congested() {
            progress_count -= 1;
            let Some(sid) = self.streams.next_parked() else {
                break;
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
                if let Err(e) = self.try_pump_streaming_body(sid) {
                    self.fail_stream(sid, e);
                }
                continue;
            }

            if let Err(e) = self.write_data_frames(sid, pending.remaining, DataEnd::Buffered) {
                self.fail_stream(sid, e);
            }
        }
    }
}
