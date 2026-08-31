//! Inbound frame handling, flow-control top-ups, and graceful shutdown.

use std::io;
use std::time::Instant;

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::core::HeaderStr;
use crate::h2::error::{ErrorCode, H2Error};
use crate::h2::stream_state::StreamEvent;

use super::*;

/// Slack permitted before an over-window inbound DATA frame is treated as a flow-control violation (RFC 9113 §6.9.1).
const RECV_WINDOW_VIOLATION_SLACK: i64 = 16 * 1024;

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) async fn on_inbound_frame(&mut self, frame: Frame) -> Result<(), H2Error> {
        let enforce_odd = |sid: u32| -> Result<(), H2Error> {
            if sid == 0 || sid % 2 == 0 {
                return Err(H2Error::Connection {
                    code: ErrorCode::ProtocolError,
                    reason: format!(
                        "peer used server-initiated stream id {sid} for a client-expected frame"
                    ),
                });
            }
            Ok(())
        };

        match frame {
            Frame::Headers(h) => {
                enforce_odd(h.stream_id)?;
                self.on_headers(h).await?
            }
            Frame::Data(d) => {
                enforce_odd(d.stream_id)?;
                self.on_data(d).await?
            }
            Frame::Settings(s) if s.ack => {}
            Frame::Settings(s) => {
                self.settings_flood.record(Instant::now())?;
                let result = self.peer_settings.apply(&s.params)?;
                if let Some(delta) = result.window_size_delta {
                    for (sid, actor) in self.streams.iter_mut() {
                        match checked_window_add(actor.send_window, delta) {
                            Ok(v) => actor.send_window = v,
                            Err(new_win) => {
                                return Err(H2Error::Connection {
                                    code: ErrorCode::FlowControlError,
                                    reason: format!(
                                        "SETTINGS_INITIAL_WINDOW_SIZE delta pushes stream {sid} window to {new_win} (> 2^31-1)"
                                    ),
                                });
                            }
                        }
                    }
                }
                self.peer_snapshot
                    .set_max_concurrent_streams(self.peer_settings.max_concurrent_streams);
                self.peer_greeted = true;
                self.drain_pending().await?;
                self.peer_snapshot
                    .set_enable_connect_protocol(self.peer_settings.enable_connect_protocol);
                self.writer.write_settings_ack().await?;
                let encoder_cap = (self.peer_settings.header_table_size as usize).min(4096);
                self.encoder.set_max_table_size(encoder_cap);
            }
            Frame::WindowUpdate(w) if w.stream_id == 0 => {
                self.conn_send_window = match checked_window_add(
                    self.conn_send_window,
                    w.increment as i64,
                ) {
                    Ok(v) => v,
                    Err(new_win) => {
                        return Err(H2Error::Connection {
                            code: ErrorCode::FlowControlError,
                            reason: format!(
                                "WINDOW_UPDATE would push connection window to {new_win} (> 2^31-1)"
                            ),
                        });
                    }
                };
            }
            Frame::WindowUpdate(w) => {
                if let Some(actor) = self.streams.get_mut(&w.stream_id) {
                    match checked_window_add(actor.send_window, w.increment as i64) {
                        Ok(v) => actor.send_window = v,
                        Err(_) => {
                            self.writer
                                .write_rst_stream(w.stream_id, ErrorCode::FlowControlError)
                                .await?;
                            let err = H2Error::Stream {
                                stream_id: w.stream_id,
                                code: ErrorCode::FlowControlError,
                            };
                            self.fail_stream(w.stream_id, err);
                        }
                    }
                }
            }
            Frame::Ping(p) if !p.ack => {
                self.writer.write_ping_ack(p.payload).await?;
            }
            Frame::Ping(_) => {}
            Frame::GoAway(g) => {
                self.peer_goaway_last_stream = Some(g.last_stream_id);
                if !matches!(g.error_code, ErrorCode::NoError) {
                    let to_fail: Vec<u32> = self
                        .streams
                        .keys()
                        .copied()
                        .filter(|sid| *sid > g.last_stream_id)
                        .collect();
                    for sid in to_fail {
                        self.fail_stream(
                            sid,
                            H2Error::Connection {
                                code: g.error_code,
                                reason: format!("peer GOAWAY: {:?}", g.error_code),
                            },
                        );
                    }
                    return Err(H2Error::Connection {
                        code: g.error_code,
                        reason: format!("server sent GOAWAY: {:?}", g.error_code),
                    });
                }
            }
            Frame::RstStream(r) => {
                if r.stream_id == 0 || (r.stream_id % 2 == 1 && r.stream_id >= self.next_stream_id)
                {
                    return Err(H2Error::Connection {
                        code: ErrorCode::ProtocolError,
                        reason: format!("peer sent RST_STREAM on idle stream id {}", r.stream_id),
                    });
                }
                self.rst_flood.record(Instant::now())?;
                if let Some(actor) = self.streams.get_mut(&r.stream_id) {
                    let _ = actor
                        .state
                        .transition(StreamEvent::RecvRstStream(r.error_code));
                }
                let err = H2Error::Stream {
                    stream_id: r.stream_id,
                    code: r.error_code,
                };
                self.fail_stream(r.stream_id, err);
            }
            Frame::PushPromise(pp) => {
                let decoded = self.decoder.decode_header_block(&pp.fragment);
                self.writer
                    .write_rst_stream(pp.promised_stream_id, ErrorCode::Cancel)
                    .await?;
                decoded.map_err(H2Error::Hpack)?;
            }
            Frame::Continuation { .. } => {
                return Err(H2Error::Connection {
                    code: ErrorCode::ProtocolError,
                    reason: "unexpected CONTINUATION".into(),
                });
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) async fn on_headers(&mut self, h: HeadersFrame) -> Result<(), H2Error> {
        let full_fragment = if h.end_headers {
            h.fragment
        } else {
            let max_header_block = self.config.max_header_block_bytes;
            let reassembly_timeout = self.config.header_block_reassembly_timeout;
            let deadline = tokio::time::Instant::now() + reassembly_timeout;
            let mut assembled = h.fragment.to_vec();
            loop {
                if assembled.len() > max_header_block {
                    return Err(H2Error::Connection {
                        code: ErrorCode::CompressionError,
                        reason: format!(
                            "header block exceeds max_header_block_bytes ({max_header_block})"
                        ),
                    });
                }
                let cont = match tokio::time::timeout_at(deadline, self.reader.next()).await {
                    Ok(inner) => inner?,
                    Err(_) => {
                        return Err(H2Error::Connection {
                            code: ErrorCode::ProtocolError,
                            reason: format!(
                                "CONTINUATION reassembly exceeded {reassembly_timeout:?}"
                            ),
                        });
                    }
                }
                .ok_or_else(|| H2Error::Connection {
                    code: ErrorCode::ProtocolError,
                    reason: "connection closed during CONTINUATION".into(),
                })?;
                match cont {
                    Frame::Continuation {
                        stream_id,
                        end_headers,
                        fragment,
                    } if stream_id == h.stream_id => {
                        assembled.extend_from_slice(&fragment);
                        if end_headers {
                            break;
                        }
                    }
                    _ => {
                        return Err(H2Error::Connection {
                            code: ErrorCode::ProtocolError,
                            reason: "expected CONTINUATION frame".into(),
                        });
                    }
                }
            }
            Bytes::from(assembled)
        };

        let decoded = self
            .decoder
            .decode_header_block(&full_fragment)
            .map_err(H2Error::Hpack)?;

        let stream_id = h.stream_id;
        let actor = match self.streams.get_mut(&stream_id) {
            Some(a) => a,
            None => return Ok(()),
        };

        if !actor.got_headers {
            if let Err(e) = actor.state.transition(StreamEvent::RecvHeaders {
                end_stream: h.end_stream,
            }) {
                let err = map_state_err(stream_id, e);
                self.fail_stream(stream_id, err);
                return Ok(());
            }
            let mut status = None;
            let mut saw_regular = false;
            let mut bad_status = false;
            for header in decoded {
                if header.name.starts_with(b":") {
                    let value = header.value.as_ref();
                    if saw_regular
                        || header.name.as_ref() != b":status"
                        || status.is_some()
                        || value.len() != 3
                        || !value.iter().all(u8::is_ascii_digit)
                    {
                        bad_status = true;
                        break;
                    }
                    status = Some(
                        value
                            .iter()
                            .fold(0_u16, |code, digit| code * 10 + u16::from(*digit - b'0')),
                    );
                } else {
                    saw_regular = true;
                    actor.resp_headers.push((
                        HeaderStr::from_bytes_lossy(header.name),
                        HeaderStr::from_bytes_lossy(header.value),
                    ));
                }
            }
            let Some(status) = status.filter(|status| !bad_status && *status != 101) else {
                self.fail_stream(
                    stream_id,
                    H2Error::Stream {
                        stream_id,
                        code: ErrorCode::ProtocolError,
                    },
                );
                return Ok(());
            };
            actor.status = status;
            if matches!(actor.status, 100..=199) && actor.status != 101 && !h.end_stream {
                actor.status = 0;
                actor.resp_headers.clear();
                return Ok(());
            }
            actor.got_headers = true;
            if matches!(actor.status, 204 | 304) {
                actor.drop_body = true;
            }
            if matches!(actor.response_tx, Some(ResponseSink::StreamingEx { .. })) {
                actor.deliver_headers_streaming();
            }
            if h.end_stream {
                self.complete_stream(stream_id);
            }
        } else {
            if !h.end_stream {
                self.fail_stream(
                    stream_id,
                    H2Error::Stream {
                        stream_id,
                        code: ErrorCode::ProtocolError,
                    },
                );
                return Ok(());
            }
            if decoded.iter().any(|header| header.name.starts_with(b":")) {
                self.fail_stream(
                    stream_id,
                    H2Error::Stream {
                        stream_id,
                        code: ErrorCode::ProtocolError,
                    },
                );
                return Ok(());
            }
            if let Err(e) = actor.state.transition(StreamEvent::RecvTrailers) {
                let err = map_state_err(stream_id, e);
                self.fail_stream(stream_id, err);
                return Ok(());
            }
            let mut th = Vec::new();
            for header in decoded {
                th.push((
                    HeaderStr::from_bytes_lossy(header.name),
                    HeaderStr::from_bytes_lossy(header.value),
                ));
            }
            actor.trailers = Some(th);
            self.complete_stream(stream_id);
        }
        Ok(())
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
        let fail_outcome: Option<(H2Error, ErrorCode)> = {
            let actor = match self.streams.get_mut(&stream_id) {
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
                self.maybe_top_up_conn_window().await?;
                return Ok(());
            }
            let mut outcome = None;
            if !actor.drop_body {
                let is_streaming =
                    matches!(actor.response_tx, Some(ResponseSink::StreamingEx { .. }));
                if is_streaming {
                    if !d.data.is_empty() {
                        if let Some(ResponseSink::StreamingEx { body_tx, .. }) =
                            actor.response_tx.as_ref()
                        {
                            match body_tx.try_send(Ok(d.data.clone())) {
                                Ok(()) => {}
                                Err(mpsc::error::TrySendError::Full(chunk)) => {
                                    let _ = body_tx.try_send(Err(io::Error::new(
                                        io::ErrorKind::WouldBlock,
                                        "streaming response consumer fell behind the peer; \
                                         stream cancelled. Raise the response body channel \
                                         capacity or read faster.",
                                    )));
                                    drop(chunk);
                                    tracing::warn!(
                                        target: "leyline::h2",
                                        stream_id,
                                        "streaming response consumer saturated; sending RST_STREAM(CANCEL)"
                                    );
                                    outcome = Some((
                                        H2Error::Stream {
                                            stream_id,
                                            code: ErrorCode::Cancel,
                                        },
                                        ErrorCode::Cancel,
                                    ));
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

        if let Some((err, code)) = fail_outcome {
            let _ = self.writer.write_rst_stream(stream_id, code).await;
            self.fail_stream(stream_id, err);
            self.maybe_top_up_conn_window().await?;
            return Ok(());
        }

        self.maybe_top_up_conn_window().await?;
        self.maybe_top_up_stream_window(stream_id).await?;

        if complete {
            self.complete_stream(stream_id);
        }
        Ok(())
    }
}
