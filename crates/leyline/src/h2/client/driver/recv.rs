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

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) async fn on_inbound_frame(&mut self, frame: Frame) -> Result<(), H2Error> {
        // RFC 9113 §5.1.1: client-initiated streams use odd identifiers;
        // server-initiated (PUSH_PROMISE) use even. Any HEADERS / DATA
        // frame from the peer on an even stream id — or on any stream
        // id the client didn't originate — is a connection error. We
        // catch this at the entry point rather than in every handler.
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
            Frame::Settings(s) if s.ack => {
                // ACK of our settings — ignored.
            }
            Frame::Settings(s) => {
                // Rate-limit inbound non-ACK SETTINGS: each one forces
                // an ACK write + a stream-window rescan, so a burst is
                // CPU-expensive. Trip `ENHANCE_YOUR_CALM` past the
                // configured threshold.
                self.settings_flood.record(Instant::now())?;
                let result = self.peer_settings.apply(&s.params)?;
                if let Some(delta) = result.window_size_delta {
                    // RFC 9113 §6.9.2: a SETTINGS_INITIAL_WINDOW_SIZE
                    // change that causes any stream flow-control
                    // window to exceed 2^31 − 1 MUST be treated as a
                    // connection error of type FLOW_CONTROL_ERROR.
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
                self.peer_snapshot
                    .set_enable_connect_protocol(self.peer_settings.enable_connect_protocol);
                self.writer.write_settings_ack().await?;
                self.reader
                    .set_max_frame_size(self.peer_settings.max_frame_size);
                // Only the encoder tracks the peer's HEADER_TABLE_SIZE (it
                // bounds how large a dynamic table *we* may push toward the
                // peer's decoder). Our decoder's ceiling is whatever we
                // advertised at connection setup and does not change when
                // the peer updates its own SETTINGS.
                self.encoder
                    .set_max_table_size(self.peer_settings.header_table_size as usize);
            }
            Frame::WindowUpdate(w) if w.stream_id == 0 => {
                // RFC 9113 §6.9.1: a sender MUST NOT allow a
                // flow-control window to exceed 2^31 − 1.
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
                    // §6.9.1 stream-level overflow — RST_STREAM
                    // with FLOW_CONTROL_ERROR rather than tearing
                    // the whole connection down.
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
                // Mark connection as no-new-streams; existing streams
                // ≤ last_stream_id may finish. If error is non-zero,
                // tear the connection down as a connection error.
                self.peer_goaway_last_stream = Some(g.last_stream_id);
                if !matches!(g.error_code, ErrorCode::NoError) {
                    // Fail all streams above last_stream_id immediately.
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
                // RFC 9113 §5.4.2 / §5.1: RST_STREAM on an "idle"
                // stream (one the client never opened) is a connection
                // error with PROTOCOL_ERROR. `stream_id == 0` is
                // reserved for the connection and must never appear
                // on RST_STREAM. An odd stream id at or above
                // `next_stream_id` is the client's own ID space but
                // has not yet been allocated — idle by definition.
                // Even stream ids are server-push reservations, which
                // we reject at PUSH_PROMISE anyway.
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
                self.writer
                    .write_rst_stream(pp.promised_stream_id, ErrorCode::Cancel)
                    .await?;
            }
            Frame::Continuation { .. } => {
                // A bare CONTINUATION without a preceding HEADERS we
                // already consumed is a protocol error.
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
        // Reassemble CONTINUATION.
        let full_fragment = if h.end_headers {
            h.fragment
        } else {
            let max_header_block = self.config.max_header_block_bytes;
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
                let cont = self
                    .reader
                    .next()
                    .await?
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
            None => return Ok(()), // Unknown stream, ignore.
        };

        if !actor.got_headers {
            if let Err(e) = actor.state.transition(StreamEvent::RecvHeaders {
                end_stream: h.end_stream,
            }) {
                let err = map_state_err(stream_id, e);
                self.fail_stream(stream_id, err);
                return Ok(());
            }
            for header in decoded {
                if header.name.as_ref() == b":status" {
                    actor.status = std::str::from_utf8(&header.value)
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .ok_or_else(|| H2Error::Hpack("invalid :status".into()))?;
                } else if !header.name.starts_with(b":") {
                    actor.resp_headers.push((
                        HeaderStr::from_utf8_unchecked(header.name),
                        HeaderStr::from_utf8_unchecked(header.value),
                    ));
                }
            }
            // 1xx informational (except 101) is provisional — discard and await
            // the real final HEADERS (mirrors H1 read_h1_response). Keep HPACK state.
            if matches!(actor.status, 100..=199) && actor.status != 101 && !h.end_stream {
                actor.status = 0;
                actor.resp_headers.clear();
                return Ok(());
            }
            actor.got_headers = true;
            if matches!(actor.status, 204 | 304) {
                actor.drop_body = true;
            }
            // Streaming-response sinks deliver headers immediately.
            if matches!(actor.response_tx, Some(ResponseSink::StreamingEx { .. })) {
                actor.deliver_headers_streaming();
            }
            if h.end_stream {
                self.complete_stream(stream_id);
            }
        } else {
            // Trailers.
            if let Err(e) = actor.state.transition(StreamEvent::RecvTrailers) {
                let err = map_state_err(stream_id, e);
                self.fail_stream(stream_id, err);
                return Ok(());
            }
            let mut th = Vec::new();
            for header in decoded {
                th.push((
                    HeaderStr::from_utf8_unchecked(header.name),
                    HeaderStr::from_utf8_unchecked(header.value),
                ));
            }
            actor.trailers = Some(th);
            self.complete_stream(stream_id);
        }
        Ok(())
    }

    pub(super) async fn on_data(&mut self, d: DataFrame) -> Result<(), H2Error> {
        let stream_id = d.stream_id;
        let len = d.data.len() as i64;

        // Connection-level flow control: the peer burned `len` bytes of
        // its `conn_send_window` to put these bytes on the wire, so we
        // must always account for them against our `conn_recv_window`
        // and eventually send a WINDOW_UPDATE, *regardless* of what
        // happens to the payload below. Early-returning on stream-local
        // errors (unknown id, bad state, max-body exceeded, consumer
        // overflow) without crediting back the conn window means that,
        // over a session with repeated slow-consumer RSTs, the peer's
        // `conn_send_window` drains to zero and every stream on the
        // connection would stall.
        self.conn_recv_window -= len;

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
                    // Forward the chunk to the consumer via the body
                    // channel. `try_send` ONLY — the driver is the
                    // single task owning reader + writer + every
                    // stream, so we must never `.await` on a consumer
                    // channel here: a slow consumer on stream A would
                    // otherwise stall every other concurrent stream
                    // on the same TCP connection, silently voiding
                    // the multiplexing guarantee the whole driver
                    // architecture exists to provide.
                    //
                    // When the consumer's bounded channel is full, we
                    // surface that as an `ErrorKind::WouldBlock` item
                    // to the consumer, and signal a stream-level RST
                    // to the main body below (which still runs the
                    // conn-window accounting).
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
            complete = d.end_stream;
            outcome
        };

        if let Some((err, code)) = fail_outcome {
            let _ = self.writer.write_rst_stream(stream_id, code).await;
            self.fail_stream(stream_id, err);
            // Still need to credit back the conn window even on
            // stream-level failure.
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
