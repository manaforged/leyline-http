use std::collections::VecDeque;
#[cfg(feature = "websocket")]
use std::io;

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};
#[cfg(feature = "websocket")]
use tokio::sync::mpsc;

use crate::h2::connection::{PseudoHeaders, encode_request_pseudos};
use crate::h2::error::{ErrorCode, H2Error};
use crate::h2::stream_state::StreamEvent;

use super::*;

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) async fn on_command(&mut self, cmd: DriverCommand) -> Result<(), H2Error> {
        if cmd.is_cancelled() {
            return Ok(());
        }
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
            #[cfg(feature = "websocket")]
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
            DriverCommand::SendRequest { head, body, sink } => {
                if let Err(e) = self.admit_new_stream() {
                    if Self::deferrable_capacity_error(&e) {
                        self.pending
                            .push_back(DriverCommand::SendRequest { head, body, sink });
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
        while !self.pending.is_empty() && self.peer_ready() {
            if let Err(e) = self.admit_new_stream()
                && Self::deferrable_capacity_error(&e)
            {
                break;
            }
            let Some(cmd) = self.pending.pop_front() else {
                break;
            };
            self.on_command(cmd).await?;
        }
        Ok(())
    }

    fn start_upload(&mut self, stream_id: u32, body: crate::util::upload::BodyStream) {
        let Some(actor) = self.streams.get_mut(stream_id) else {
            return;
        };
        let credit = crate::util::upload::upload_credit();
        let pump = tokio::spawn(crate::util::upload::pump_request_body(
            stream_id,
            body,
            self.body_chunk_tx.clone(),
            Arc::clone(&credit),
        ));
        actor.upload_credit = Some(credit);
        actor.pump = Some(pump.abort_handle());
    }

    pub(super) async fn start_request(
        &mut self,
        pseudo: &PseudoHeaders,
        headers: &[crate::h2::connection::HeaderPair],
        body: Option<Bytes>,
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

        let end_stream_on_headers = body.is_none();

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

        if let Some(body) = body
            && let Err(e) = self.write_body_or_park(stream_id, body).await
        {
            self.fail_stream(stream_id, e);
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
            DriverRequestBody::None => self.start_request(pseudo, headers, None, sink).await,
            DriverRequestBody::Buffered(b) => {
                let body = if b.is_empty() { None } else { Some(b) };
                self.start_request(pseudo, headers, body, sink).await
            }
            DriverRequestBody::Streaming(body) => {
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

                self.start_upload(stream_id, body);
                Ok(())
            }
        }
    }

    #[cfg(feature = "websocket")]
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

        self.start_upload(
            stream_id,
            Box::pin(futures_util::stream::unfold(
                write_rx,
                |mut rx| async move { rx.recv().await.map(|item| (item, rx)) },
            )),
        );
        Ok(())
    }
}
