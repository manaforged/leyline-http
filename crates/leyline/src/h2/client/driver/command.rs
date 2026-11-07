use std::collections::VecDeque;

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::h2::stream_state::StreamEvent;
use crate::wire::HeaderPair;
use crate::wire::{Body, BodyKind, BoxedStream};

use super::lifecycle::Admit;
use super::send::DataEnd;
use super::*;

pub(super) enum Upload {
    None,
    Bytes(Bytes),
    Stream(BoxedStream),
}

impl From<Body> for Upload {
    fn from(body: Body) -> Self {
        match body.0 {
            BodyKind::Empty => Self::None,
            BodyKind::Bytes(bytes) if bytes.is_empty() => Self::None,
            BodyKind::Bytes(bytes) => Self::Bytes(bytes),
            BodyKind::Stream { stream, .. } => Self::Stream(stream),
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) fn on_command(&mut self, cmd: DriverCommand) {
        if cmd.is_cancelled() {
            return;
        }
        if !self.peer_greeted {
            self.pending.push_back(cmd);
            return;
        }
        match cmd {
            DriverCommand::Ping(ping) => self.send_ping(ping),
            #[cfg(feature = "websocket")]
            DriverCommand::OpenConnect {
                head,
                write_rx,
                sink,
                slot,
            } if self.peer_settings.enable_connect_protocol => match self.admit() {
                Admit::Open => {
                    let upload = Upload::Stream(Box::pin(futures_util::stream::unfold(
                        write_rx,
                        |mut rx| async move { rx.recv().await.map(|item| (item, rx)) },
                    )));
                    self.open_stream(&head, upload, sink);
                    drop(slot);
                }
                Admit::Defer => self.pending.push_back(DriverCommand::OpenConnect {
                    head,
                    write_rx,
                    sink,
                    slot,
                }),
                Admit::Refuse(e) => sink.fail(e),
            },
            #[cfg(feature = "websocket")]
            DriverCommand::OpenConnect { sink, .. } => sink.fail(H2Error::NoExtendedConnect),
            DriverCommand::SendRequest {
                head,
                body,
                sink,
                slot,
            } => match self.admit() {
                Admit::Open => {
                    self.open_stream(&head, Upload::from(body), sink);
                    drop(slot);
                }
                Admit::Defer => self.pending.push_back(DriverCommand::SendRequest {
                    head,
                    body,
                    sink,
                    slot,
                }),
                Admit::Refuse(e) => sink.fail(e),
            },
        }
    }

    pub(super) fn drain_pending(&mut self) {
        while !self.pending.is_empty() && self.peer_greeted {
            if matches!(self.admit(), Admit::Defer) {
                break;
            }
            let Some(cmd) = self.pending.pop_front() else {
                break;
            };
            self.on_command(cmd);
        }
    }

    fn start_upload(&mut self, stream_id: u32, body: BoxedStream) {
        let Some(actor) = self.streams.get_mut(stream_id) else {
            return;
        };
        let (tx, _) = self
            .body_chunks
            .get_or_insert_with(|| tokio::sync::mpsc::channel(STREAM_REQ_BODY_CAPACITY * 4));
        actor.upload = Some(crate::wire::Upload::start(stream_id, body, tx.clone()));
    }

    fn open_stream(&mut self, head: &Head, upload: Upload, sink: ResponseSink) {
        let stream_id = self.alloc_stream_id();
        let (pseudo_list, pseudo_len) =
            match head.pseudo.build_pseudo_list(&self.config.pseudo_order) {
                Ok(list) => list,
                Err(e) => {
                    sink.fail(e);
                    return;
                }
            };
        let pseudo = &pseudo_list[..pseudo_len];
        let headers: &[HeaderPair] = &head.headers;
        if let Err(e) = self.check_header_list(pseudo, headers) {
            sink.fail(e);
            return;
        }

        let end_stream = matches!(upload, Upload::None);
        let initial_send = self.peer_settings.initial_window_size as i64;
        let initial_recv = self.config.advertised_initial_window_size() as i64;
        let is_head = head.pseudo.method_is(&http::Method::HEAD);
        let mut actor =
            StreamActor::new(initial_send, initial_recv, sink, is_head, head.body_limit);
        if matches!(upload, Upload::Stream(_)) {
            actor.send_body_input = SendBodyInput::Streaming {
                pending_buf: VecDeque::new(),
                closed: false,
                error: None,
            };
        }
        if let Err(e) = actor
            .state
            .transition(StreamEvent::SendHeaders { end_stream })
        {
            actor.deliver_err(map_state_err(stream_id, e));
            return;
        }

        self.streams.insert(stream_id, actor);
        self.write_headers_block(stream_id, end_stream, (pseudo, headers));

        match upload {
            Upload::None => {}
            Upload::Bytes(body) => {
                if let Err(e) = self.write_data_frames(stream_id, body, DataEnd::Buffered) {
                    self.fail_stream(stream_id, e);
                }
            }
            Upload::Stream(body) => self.start_upload(stream_id, body),
        }
    }
}
