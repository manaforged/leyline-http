use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::h2::error::ErrorCode;

use crate::h2::H2Error;
use crate::h2::frame::U31_MAX;
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

impl StreamActor {
    fn forward_chunk(&mut self, d: &DataFrame) -> Result<usize, Failure> {
        let Some(ResponseSink::StreamingEx { body_tx, .. }) = self.response_tx.as_ref() else {
            return Ok(0);
        };
        if d.data.is_empty() {
            return Ok(0);
        }
        let sent = if self.stalled.is_empty() {
            body_tx.try_send(Ok(d.data.clone()))
        } else {
            Err(mpsc::error::TrySendError::Full(Ok(d.data.clone())))
        };
        match sent {
            Ok(()) => Ok(0),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.stalled.push_back(d.data.clone());
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

    fn buffer_chunk(&mut self, d: &DataFrame) -> Result<usize, Failure> {
        self.body_limit
            .admit(self.body.len(), d.data.len())
            .map_err(|limit| (H2Error::BodyLimit(limit), ErrorCode::Cancel))?;
        self.body.extend_from_slice(&d.data);
        Ok(0)
    }

    fn store_chunk(&mut self, d: &DataFrame) -> Result<usize, Failure> {
        if !self.has_body {
            Ok(0)
        } else if matches!(self.response_tx, Some(ResponseSink::StreamingEx { .. })) {
            self.forward_chunk(d)
        } else {
            self.buffer_chunk(d)
        }
    }

    fn charge_stream_window(&mut self, stream_id: u32, len: i64) -> Option<Failure> {
        self.recv_window -= len;
        if self.recv_window >= -RECV_WINDOW_VIOLATION_SLACK {
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

    fn accept_data(&mut self, d: &DataFrame, len: i64) -> Intake {
        if self.response.data().is_err() {
            self.recv_window -= len;
            return Intake::Taken {
                queued: 0,
                failure: Some((
                    H2Error::Stream {
                        stream_id: d.stream_id,
                        code: ErrorCode::ProtocolError,
                    },
                    ErrorCode::ProtocolError,
                )),
            };
        }
        let event = StreamEvent::RecvData {
            end_stream: d.end_stream,
        };
        if let Err(e) = self.state.transition(event) {
            self.recv_window -= len;
            return Intake::Illegal(map_state_err(d.stream_id, e));
        }
        self.recv_len = self.recv_len.saturating_add(d.data.len() as u64);
        let stored = self.store_chunk(d);
        let overrun = self.charge_stream_window(d.stream_id, len);
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
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) async fn on_inbound_frame(&mut self, frame: Frame) -> Result<(), H2Error> {
        if !self.peer_greeted && !matches!(&frame, Frame::Settings(s) if !s.ack) {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "server preface must start with non-ACK SETTINGS".into(),
            });
        }
        let stream_id = match &frame {
            Frame::Headers(h) => h.stream_id,
            Frame::Data(d) => d.stream_id,
            Frame::RstStream(r) => r.stream_id,
            Frame::WindowUpdate(w) => w.stream_id,
            _ => 0,
        };
        if stream_id % 2 == 1 && stream_id >= self.next_stream_id {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: format!("peer referenced idle stream id {stream_id}"),
            });
        }
        match frame {
            Frame::Headers(h) => {
                enforce_odd(h.stream_id)?;
                self.on_headers(h).await
            }
            Frame::Data(d) => {
                enforce_odd(d.stream_id)?;
                self.on_data(d)
            }
            Frame::Settings(s) => self.on_settings(s),
            Frame::WindowUpdate(w) => self.on_window(w),
            Frame::Ping(p) => self.on_ping(p),
            Frame::GoAway(g) => self.on_goaway(g),
            Frame::RstStream(r) => self.on_rst(r),
            Frame::PushPromise(pp) => self.on_push(pp),
            Frame::Continuation { .. } => Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "unexpected CONTINUATION".into(),
            }),
            Frame::Priority | Frame::Unknown => Ok(()),
        }
    }

    pub(super) fn on_data(&mut self, d: DataFrame) -> Result<(), H2Error> {
        let stream_id = d.stream_id;
        let len = d.wire_len as i64;
        self.charge_conn_window(len)?;
        match self.receive_data(&d, len) {
            Intake::NoStream => {
                self.maybe_top_up_conn_window();
                Ok(())
            }
            Intake::Illegal(err) => self.reject_data(stream_id, err),
            Intake::Taken { queued, failure } => {
                self.streams.add_stalled(queued);
                match failure {
                    Some((err, code)) => {
                        self.reset_stream(stream_id, err, code)?;
                        self.maybe_top_up_conn_window();
                        Ok(())
                    }
                    None => self.settle_data(stream_id, d.end_stream),
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
        match self.streams.get_mut(d.stream_id) {
            Some(actor) if !actor.state.is_reset_local() => actor.accept_data(d, len),
            Some(_) => Intake::NoStream,
            None => Intake::NoStream,
        }
    }

    fn reject_data(&mut self, stream_id: u32, err: H2Error) -> Result<(), H2Error> {
        self.reset_stream(stream_id, err, ErrorCode::StreamClosed)?;
        self.maybe_top_up_conn_window();
        Ok(())
    }

    fn settle_data(&mut self, stream_id: u32, end_stream: bool) -> Result<(), H2Error> {
        self.maybe_top_up_conn_window();
        self.maybe_top_up_stream_window(stream_id);
        if end_stream {
            self.finish_remote(stream_id)?;
        }
        Ok(())
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) fn maybe_top_up_conn_window(&mut self) {
        let initial = self.config.initial_connection_window_size as i64;
        if let Some(increment) = refill(initial, self.conn_recv_window) {
            WindowUpdateFrame {
                stream_id: 0,
                increment,
            }
            .encode(self.writer.out());
            self.conn_recv_window += increment as i64;
        }
    }

    pub(super) fn maybe_top_up_stream_window(&mut self, stream_id: u32) {
        let initial = self.config.advertised_initial_window_size() as i64;
        let increment = self
            .streams
            .get(stream_id)
            .filter(|a| a.stalled.is_empty())
            .and_then(|a| refill(initial, a.recv_window));
        if let Some(increment) = increment {
            WindowUpdateFrame {
                stream_id,
                increment,
            }
            .encode(self.writer.out());
            if let Some(actor) = self.streams.get_mut(stream_id) {
                actor.recv_window += increment as i64;
            }
        }
    }
}

fn refill(initial: i64, current: i64) -> Option<u32> {
    (current < initial / 2).then(|| (initial - current).clamp(1, i64::from(U31_MAX)) as u32)
}
