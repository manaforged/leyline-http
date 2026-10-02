use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::core::session::decompress::BodyLimit;
use crate::h2::error::{ErrorCode, H2Error};
use crate::h2::stream_state::StreamEvent;

use super::*;

mod frames;
mod headers;

const RECV_WINDOW_VIOLATION_SLACK: i64 = 16 * 1024;

type Failure = (H2Error, ErrorCode);

enum Intake {
    NoStream,
    Illegal(H2Error),
    Taken {
        queued: usize,
        failure: Option<Failure>,
    },
}

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

fn forward_chunk(actor: &mut StreamActor, d: &DataFrame) -> Result<usize, Failure> {
    let Some(ResponseSink::StreamingEx { body_tx, .. }) = actor.response_tx.as_ref() else {
        return Ok(0);
    };
    if d.data.is_empty() {
        return Ok(0);
    }
    let sent = if actor.stalled.is_empty() {
        body_tx.try_send(Ok(d.data.clone()))
    } else {
        Err(mpsc::error::TrySendError::Full(Ok(d.data.clone())))
    };
    match sent {
        Ok(()) => Ok(0),
        Err(mpsc::error::TrySendError::Full(_)) => {
            actor.stalled.push_back(d.data.clone());
            Ok(1)
        }
        Err(mpsc::error::TrySendError::Closed(_)) => Err((
            H2Error::Stream {
                stream_id: d.stream_id,
                code: ErrorCode::Cancel,
            },
            ErrorCode::Cancel,
        )),
    }
}

fn buffer_chunk(actor: &mut StreamActor, d: &DataFrame, max_body: usize) -> Result<usize, Failure> {
    if actor.body.len() + d.data.len() > max_body {
        return Err((
            H2Error::Io(BodyLimit::session(max_body).into_io()),
            ErrorCode::Cancel,
        ));
    }
    actor.body.extend_from_slice(&d.data);
    Ok(0)
}

fn store_chunk(actor: &mut StreamActor, d: &DataFrame, max_body: usize) -> Result<usize, Failure> {
    if actor.drop_body {
        Ok(0)
    } else if matches!(actor.response_tx, Some(ResponseSink::StreamingEx { .. })) {
        forward_chunk(actor, d)
    } else {
        buffer_chunk(actor, d, max_body)
    }
}

fn charge_stream_window(actor: &mut StreamActor, stream_id: u32, len: i64) -> Option<Failure> {
    actor.recv_window -= len;
    if actor.recv_window >= -RECV_WINDOW_VIOLATION_SLACK {
        return None;
    }
    Some((
        H2Error::Stream {
            stream_id,
            code: ErrorCode::FlowControlError,
        },
        ErrorCode::FlowControlError,
    ))
}

fn accept_data(actor: &mut StreamActor, d: &DataFrame, len: i64, max_body: usize) -> Intake {
    let event = StreamEvent::RecvData {
        end_stream: d.end_stream,
    };
    if let Err(e) = actor.state.transition(event) {
        actor.recv_window -= len;
        return Intake::Illegal(map_state_err(d.stream_id, e));
    }
    actor.recv_len = actor.recv_len.saturating_add(d.data.len() as u64);
    let stored = store_chunk(actor, d, max_body);
    let overrun = charge_stream_window(actor, d.stream_id, len);
    match stored {
        Ok(queued) => Intake::Taken {
            queued,
            failure: overrun,
        },
        Err(failure) => Intake::Taken {
            queued: 0,
            failure: Some(failure),
        },
    }
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
        self.charge_conn_window(len)?;
        match self.receive_data(&d, len) {
            Intake::NoStream => self.maybe_top_up_conn_window().await,
            Intake::Illegal(err) => self.reject_data(stream_id, err).await,
            Intake::Taken { queued, failure } => {
                self.stalled += queued;
                match failure {
                    Some((err, code)) => self.abort_data(stream_id, err, code).await,
                    None => self.settle_data(stream_id, d.end_stream).await,
                }
            }
        }
    }

    fn charge_conn_window(&mut self, len: i64) -> Result<(), H2Error> {
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
        Ok(())
    }

    fn receive_data(&mut self, d: &DataFrame, len: i64) -> Intake {
        let max_body = self.config.max_response_body_bytes;
        match self.streams.get_mut(d.stream_id) {
            Some(actor) => accept_data(actor, d, len, max_body),
            None => Intake::NoStream,
        }
    }

    async fn reject_data(&mut self, stream_id: u32, err: H2Error) -> Result<(), H2Error> {
        self.fail_stream(stream_id, err);
        let _ = self
            .writer
            .write_rst_stream(stream_id, ErrorCode::StreamClosed)
            .await;
        self.maybe_top_up_conn_window().await
    }

    async fn abort_data(
        &mut self,
        stream_id: u32,
        err: H2Error,
        code: ErrorCode,
    ) -> Result<(), H2Error> {
        let _ = self.writer.write_rst_stream(stream_id, code).await;
        self.fail_stream(stream_id, err);
        self.maybe_top_up_conn_window().await
    }

    async fn settle_data(&mut self, stream_id: u32, end_stream: bool) -> Result<(), H2Error> {
        self.maybe_top_up_conn_window().await?;
        self.maybe_top_up_stream_window(stream_id).await?;
        if end_stream {
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
