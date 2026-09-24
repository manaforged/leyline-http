use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::h2::error::{ErrorCode, H2Error};
use crate::h2::stream_state::StreamEvent;

use super::*;

mod frames;
mod headers;

const RECV_WINDOW_VIOLATION_SLACK: i64 = 16 * 1024;

fn enforce_odd(sid: u32) -> Result<(), H2Error> {
    if sid == 0 || sid.is_multiple_of(2) {
        return Err(H2Error::Connection {
            code: ErrorCode::ProtocolError,
            reason: format!(
                "peer used server-initiated stream id {sid} for a client-expected frame"
            ),
        });
    }
    Ok(())
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) async fn on_inbound_frame(&mut self, frame: Frame) -> Result<(), H2Error> {
        match frame {
            Frame::Headers(h) => {
                enforce_odd(h.stream_id)?;
                self.on_headers(h).await
            }
            Frame::Data(d) => {
                enforce_odd(d.stream_id)?;
                self.on_data(d).await
            }
            Frame::Settings(s) => self.on_settings(s).await,
            Frame::WindowUpdate(w) => self.on_window(w).await,
            Frame::Ping(p) => self.on_ping(p).await,
            Frame::GoAway(g) => self.on_goaway(g),
            Frame::RstStream(r) => self.on_rst(r),
            Frame::PushPromise(pp) => self.on_push(pp).await,
            Frame::Continuation { .. } => Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "unexpected CONTINUATION".into(),
            }),
            Frame::Priority | Frame::Unknown => Ok(()),
        }
    }

    pub(super) async fn on_data(&mut self, d: DataFrame) -> Result<(), H2Error> {
        let stream_id = d.stream_id;
        let len = d.wire_len as i64;

        self.conn_recv_window -= len;
        if self.conn_recv_window < -RECV_WINDOW_VIOLATION_SLACK {
            return Err(H2Error::Connection {
                code: ErrorCode::FlowControlError,
                reason: format!(
                    "peer overran connection receive window by {} bytes",
                    -self.conn_recv_window
                ),
            });
        }

        let complete;
        let mut queued = 0usize;
        let fail_outcome: Option<(H2Error, ErrorCode)> = {
            let actor = match self.streams.get_mut(stream_id) {
                Some(a) => a,
                None => {
                    self.maybe_top_up_conn_window().await?;
                    return Ok(());
                }
            };
            if let Err(e) = actor.state.transition(StreamEvent::RecvData {
                end_stream: d.end_stream,
            }) {
                let err = map_state_err(stream_id, e);
                actor.recv_window -= len;
                self.fail_stream(stream_id, err);
                let _ = self
                    .writer
                    .write_rst_stream(stream_id, ErrorCode::StreamClosed)
                    .await;
                self.maybe_top_up_conn_window().await?;
                return Ok(());
            }
            let mut outcome = None;
            actor.recv_len = actor.recv_len.saturating_add(d.data.len() as u64);
            if !actor.drop_body {
                let is_streaming =
                    matches!(actor.response_tx, Some(ResponseSink::StreamingEx { .. }));
                if is_streaming {
                    if !d.data.is_empty()
                        && let Some(ResponseSink::StreamingEx { body_tx, .. }) =
                            actor.response_tx.as_ref()
                    {
                        let backed_up = !actor.stalled.is_empty();
                        let sent = if backed_up {
                            Err(mpsc::error::TrySendError::Full(Ok(d.data.clone())))
                        } else {
                            body_tx.try_send(Ok(d.data.clone()))
                        };
                        match sent {
                            Ok(()) => {}
                            Err(mpsc::error::TrySendError::Full(_)) => {
                                actor.stalled.push_back(d.data.clone());
                                queued += 1;
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => {
                                outcome = Some((
                                    H2Error::Stream {
                                        stream_id,
                                        code: ErrorCode::Cancel,
                                    },
                                    ErrorCode::Cancel,
                                ));
                            }
                        }
                    }
                } else {
                    let max_body = self.config.max_response_body_bytes;
                    if actor.body.len() + d.data.len() > max_body {
                        outcome = Some((
                            H2Error::Stream {
                                stream_id,
                                code: ErrorCode::Cancel,
                            },
                            ErrorCode::Cancel,
                        ));
                    } else {
                        actor.body.extend_from_slice(&d.data);
                    }
                }
            }
            actor.recv_window -= len;
            if outcome.is_none() && actor.recv_window < -RECV_WINDOW_VIOLATION_SLACK {
                outcome = Some((
                    H2Error::Stream {
                        stream_id,
                        code: ErrorCode::FlowControlError,
                    },
                    ErrorCode::FlowControlError,
                ));
            }
            complete = d.end_stream;
            outcome
        };

        self.stalled += queued;

        if let Some((err, code)) = fail_outcome {
            let _ = self.writer.write_rst_stream(stream_id, code).await;
            self.fail_stream(stream_id, err);
            self.maybe_top_up_conn_window().await?;
            return Ok(());
        }

        self.maybe_top_up_conn_window().await?;
        self.maybe_top_up_stream_window(stream_id).await?;

        if complete {
            self.finish_remote(stream_id).await?;
        }
        Ok(())
    }
}

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
