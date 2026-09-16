use std::collections::VecDeque;
use std::io;

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::h2::connection::{PseudoHeaders, encode_request_pseudos};
use crate::h2::error::{ErrorCode, H2Error};
use crate::h2::stream_state::StreamEvent;

use super::*;

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) async fn on_command(&mut self, cmd: DriverCommand) -> Result<(), H2Error> {
        if !self.peer_ready() {
            self.pending.push_back(cmd);
            return Ok(());
        }
        match cmd {
            DriverCommand::Ping { ack_tx } => {
                self.ping_seq = self.ping_seq.wrapping_add(1);
                let payload = self.ping_seq.to_be_bytes();
                self.writer.write_ping(payload).await?;
                self.pings.push_back((payload, ack_tx));
                Ok(())
            }
            DriverCommand::SendRequest {
                pseudo,
                headers,
                body,
                trailers,
                response_tx,
            } => {
                if let Err(e) = self.admit_new_stream() {
                    if Self::deferrable_capacity_error(&e) {
                        self.pending.push_back(DriverCommand::SendRequest {
                            pseudo,
                            headers,
                            body,
                            trailers,
                            response_tx,
                        });
                        return Ok(());
                    }
                    let _ = response_tx.send(Err(e));
                    return Ok(());
                }
                self.start_request(
                    &pseudo,
                    &headers,
                    body,
                    trailers,
                    ResponseSink::Buffered(response_tx),
                )
                .await
            }
            DriverCommand::OpenConnect {
                pseudo,
                headers,
                write_rx,
                sink,
            } => {
                if let Err(e) = self.reject_after_goaway() {
                    send_err_to_sink(sink, e);
                    return Ok(());
                }
                if !self.peer_settings.enable_connect_protocol {
                    send_err_to_sink(
                        sink,
                        H2Error::Connection {
                            code: ErrorCode::ProtocolError,
                            reason: "peer does not advertise ENABLE_CONNECT_PROTOCOL".into(),
                        },
                    );
                    return Ok(());
                }
                if let Err(e) = self.check_stream_capacity() {
                    send_err_to_sink(sink, e);
                    return Ok(());
                }
                self.start_extended_connect(&pseudo, &headers, write_rx, sink)
                    .await
            }
            DriverCommand::SendRequestEx { head, body, sink } => {
                let deferrable = !matches!(
                    body,
                    crate::h2::client::driver::protocol::DriverRequestBody::Streaming { .. }
                );
                if let Err(e) = self.admit_new_stream() {
                    if Self::deferrable_capacity_error(&e) && deferrable {
                        self.pending
                            .push_back(DriverCommand::SendRequestEx { head, body, sink });
                        return Ok(());
                    }
                    send_err_to_sink(sink, e);
                    return Ok(());
                }
                self.start_request_ex(&head.pseudo, &head.headers, body, sink)
                    .await
            }
        }
    }

    fn deferrable_capacity_error(e: &H2Error) -> bool {
        matches!(
            e,
            H2Error::Connection {
                code: ErrorCode::RefusedStream,
                ..
            }
        )
    }

    pub(super) async fn drain_pending(&mut self) -> Result<(), H2Error> {
        while !self.pending.is_empty() && self.peer_ready() && self.admit_new_stream().is_ok() {
            if let Some(cmd) = self.pending.pop_front() {
                self.on_command(cmd).await?;
            }
        }
        Ok(())
    }

    pub(super) async fn start_request(
        &mut self,
        pseudo: &PseudoHeaders,
        headers: &[crate::h2::connection::HeaderPair],
        body: Option<Bytes>,
        trailers: Vec<(String, String)>,
        sink: ResponseSink,
    ) -> Result<(), H2Error> {
        let stream_id = self.alloc_stream_id();
        let is_head = pseudo.method.eq_ignore_ascii_case("HEAD");

        let (pseudo_list, pseudo_len) = match pseudo.build_pseudo_list(&self.config.pseudo_order) {
            Ok(l) => l,
            Err(e) => {
                send_err_to_sink(sink, e);
                return Ok(());
            }
        };
        let fragment =
            encode_request_pseudos(&mut self.encoder, &pseudo_list[..pseudo_len], headers);

        let has_trailers = !trailers.is_empty();
        let end_stream_on_headers = body.is_none() && !has_trailers;

        let initial_send = self.peer_settings.initial_window_size as i64;
        let initial_recv = self.config.advertised_initial_window_size() as i64;
        let mut actor = StreamActor::new(initial_send, initial_recv, sink, is_head);

        if let Err(e) = actor.state.transition(StreamEvent::SendHeaders {
            end_stream: end_stream_on_headers,
        }) {
            let err = map_state_err(stream_id, e);
            actor.deliver_err(err);
            return Ok(());
        }

        self.streams.insert(stream_id, actor);

        if let Err(e) = self
            .write_headers_block(stream_id, end_stream_on_headers, fragment, true)
            .await
        {
            self.fail_stream(stream_id, e);
            return Ok(());
        }

        let had_body = body.is_some();
        if let Some(body) = body
            && let Err(e) = self
                .write_body_or_park(stream_id, body, has_trailers, trailers.clone())
                .await
        {
            self.fail_stream(stream_id, e);
            return Ok(());
        }

        if !had_body
            && has_trailers
            && let Err(e) = self.write_trailers(stream_id, trailers).await
        {
            self.fail_stream(stream_id, e);
            return Ok(());
        }

        Ok(())
    }

    pub(super) async fn start_request_ex(
        &mut self,
        pseudo: &PseudoHeaders,
        headers: &[crate::h2::connection::HeaderPair],
        body: DriverRequestBody,
        sink: ResponseSink,
    ) -> Result<(), H2Error> {
        match body {
            DriverRequestBody::None => {
                self.start_request(pseudo, headers, None, Vec::new(), sink)
                    .await
            }
            DriverRequestBody::Buffered(b) => {
                let body = if b.is_empty() { None } else { Some(b) };
                self.start_request(pseudo, headers, body, Vec::new(), sink)
                    .await
            }
            DriverRequestBody::Streaming { rx, length_hint: _ } => {
                let stream_id = self.alloc_stream_id();
                let is_head = pseudo.method.eq_ignore_ascii_case("HEAD");

                let (pseudo_list, pseudo_len) =
                    match pseudo.build_pseudo_list(&self.config.pseudo_order) {
                        Ok(l) => l,
                        Err(e) => {
                            send_err_to_sink(sink, e);
                            return Ok(());
                        }
                    };
                let fragment =
                    encode_request_pseudos(&mut self.encoder, &pseudo_list[..pseudo_len], headers);

                let initial_send = self.peer_settings.initial_window_size as i64;
                let initial_recv = self.config.advertised_initial_window_size() as i64;
                let mut actor = StreamActor::new(initial_send, initial_recv, sink, is_head);
                actor.send_body_input = SendBodyInput::Streaming {
                    pending_buf: VecDeque::new(),
                    closed: false,
                    error: None,
                    trailers: Vec::new(),
                };

                if let Err(e) = actor
                    .state
                    .transition(StreamEvent::SendHeaders { end_stream: false })
                {
                    let err = map_state_err(stream_id, e);
                    actor.deliver_err(err);
                    return Ok(());
                }

                self.streams.insert(stream_id, actor);

                if let Err(e) = self
                    .write_headers_block(stream_id, false, fragment, true)
                    .await
                {
                    self.fail_stream(stream_id, e);
                    return Ok(());
                }

                let chunk_tx = self.body_chunk_tx.clone();
                tokio::spawn(super::bootstrap::relay_request_body(
                    stream_id, rx, chunk_tx,
                ));

                Ok(())
            }
        }
    }

    pub(super) async fn start_extended_connect(
        &mut self,
        pseudo: &PseudoHeaders,
        headers: &[crate::h2::connection::HeaderPair],
        write_rx: mpsc::Receiver<io::Result<Bytes>>,
        sink: ResponseSink,
    ) -> Result<(), H2Error> {
        let stream_id = self.alloc_stream_id();

        let (pseudo_list, pseudo_len) = match pseudo.build_pseudo_list(&self.config.pseudo_order) {
            Ok(l) => l,
            Err(e) => {
                send_err_to_sink(sink, e);
                return Ok(());
            }
        };
        let fragment =
            encode_request_pseudos(&mut self.encoder, &pseudo_list[..pseudo_len], headers);

        let initial_send = self.peer_settings.initial_window_size as i64;
        let initial_recv = self.config.advertised_initial_window_size() as i64;
        let mut actor = StreamActor::new(initial_send, initial_recv, sink, false);
        actor.send_body_input = SendBodyInput::Streaming {
            pending_buf: VecDeque::new(),
            closed: false,
            error: None,
            trailers: Vec::new(),
        };

        if let Err(e) = actor
            .state
            .transition(StreamEvent::SendHeaders { end_stream: false })
        {
            let err = map_state_err(stream_id, e);
            actor.deliver_err(err);
            return Ok(());
        }

        self.streams.insert(stream_id, actor);

        if let Err(e) = self
            .write_headers_block(stream_id, false, fragment, true)
            .await
        {
            self.fail_stream(stream_id, e);
            return Ok(());
        }

        let chunk_tx = self.body_chunk_tx.clone();
        tokio::spawn(super::bootstrap::relay_request_body(
            stream_id, write_rx, chunk_tx,
        ));

        Ok(())
    }
}
