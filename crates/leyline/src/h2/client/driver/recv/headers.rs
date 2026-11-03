use bytes::Bytes;
use http::HeaderMap;

use crate::core::deadline::{Elapsed, within};
use crate::core::transport::{ResponseHead, response_field};

use super::*;

const BODY_RESERVE_CAP: usize = 64 * 1024;

impl StreamActor {
    fn reserve_body(&mut self, cap: usize) {
        if self.drop_body {
            return;
        }
        if matches!(self.response_tx, Some(ResponseSink::StreamingEx { .. })) {
            return;
        }
        if let Some(len) = self.declared_len {
            let len = usize::try_from(len).unwrap_or(usize::MAX);
            self.body.reserve(len.min(cap).min(BODY_RESERVE_CAP));
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) async fn on_headers(&mut self, h: HeadersFrame) -> Result<(), H2Error> {
        let full_fragment = self.reassemble(&h).await?;

        let Some(response) = self
            .streams
            .get(h.stream_id)
            .filter(|actor| !actor.state.is_reset_local())
            .map(|actor| actor.response)
        else {
            return self
                .decoder
                .decode_block(&full_fragment, |_, _, _| {})
                .map_err(H2Error::Hpack);
        };
        if matches!(response, ResponseState::Initial(_)) {
            let mut head = ResponseHead::default();
            let mut malformed = false;
            self.decoder
                .decode_block(&full_fragment, |name, value, typed| {
                    if name.starts_with(b":") {
                        malformed |= head.pseudo(name, value).is_err();
                    } else {
                        malformed |= head.field(response_field(name, value, typed)).is_err();
                    }
                })
                .map_err(H2Error::Hpack)?;
            let head = if malformed { None } else { head.finish().ok() };
            self.respond(&h, head)
        } else {
            let mut trailers = HeaderMap::new();
            let mut malformed = false;
            self.decoder
                .decode_block(&full_fragment, |name, value, typed| {
                    if name.starts_with(b":") {
                        malformed = true;
                    } else {
                        match response_field(name, value, typed) {
                            Ok((name, value)) => {
                                trailers.append(name, value);
                            }
                            Err(_) => malformed = true,
                        }
                    }
                })
                .map_err(H2Error::Hpack)?;
            self.on_trailers(&h, trailers, malformed)
        }
    }

    async fn reassemble(&mut self, h: &HeadersFrame) -> Result<Bytes, H2Error> {
        let max_header_block = self.config.max_header_block_bytes;
        let oversized = || H2Error::Connection {
            code: ErrorCode::CompressionError,
            reason: format!("header block exceeds max_header_block_bytes ({max_header_block})"),
        };
        if h.fragment.len() > max_header_block {
            return Err(oversized());
        }
        if h.end_headers {
            return Ok(h.fragment.clone());
        }
        let reassembly_timeout = self.config.header_block_reassembly_timeout;
        let started = tokio::time::Instant::now();
        let mut assembled = h.fragment.to_vec();
        loop {
            let remaining = reassembly_timeout.saturating_sub(started.elapsed());
            match self
                .next_continuation(remaining, reassembly_timeout)
                .await?
            {
                Frame::Continuation {
                    stream_id,
                    end_headers,
                    fragment,
                } if stream_id == h.stream_id => {
                    if fragment.is_empty() && !end_headers {
                        self.control_flood.record(std::time::Instant::now())?;
                    }
                    if fragment.len() > max_header_block - assembled.len() {
                        return Err(oversized());
                    }
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
        Ok(Bytes::from(assembled))
    }

    async fn next_continuation(
        &mut self,
        remaining: std::time::Duration,
        limit: std::time::Duration,
    ) -> Result<Frame, H2Error> {
        let io = &mut self.io;
        let reader = &mut self.reader;
        let writer = &mut self.writer;
        let output = &mut self.output;
        let read_while_writing = async {
            loop {
                let event = std::future::poll_fn(|cx| {
                    let (writing, reading) = (writer.has_output(), !writer.saturated());
                    super::super::event::poll_io(io, writer, reader, writing, reading, cx)
                })
                .await;
                match event {
                    super::super::event::Io::Written(written) => output.wrote(written)?,
                    super::super::event::Io::Read(frame) => return frame,
                }
            }
        };
        let next = within(Some(remaining), read_while_writing)
            .await
            .map_err(|Elapsed| H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: format!("CONTINUATION reassembly exceeded {limit:?}"),
            })??;
        next.ok_or_else(|| H2Error::Connection {
            code: ErrorCode::ProtocolError,
            reason: "connection closed during CONTINUATION".into(),
        })
    }

    fn respond(
        &mut self,
        h: &HeadersFrame,
        head: Option<(http::StatusCode, HeaderMap, Option<u64>)>,
    ) -> Result<(), H2Error> {
        let stream_id = h.stream_id;
        let Some(actor) = self.streams.get_mut(stream_id) else {
            return Ok(());
        };
        let Some((status, headers, length)) = head else {
            return self.reset_stream(
                stream_id,
                H2Error::Stream {
                    stream_id,
                    code: ErrorCode::ProtocolError,
                },
                ErrorCode::ProtocolError,
            );
        };
        if (status.is_informational() && h.end_stream) || actor.response.head(status).is_err() {
            return self.reset_stream(
                stream_id,
                H2Error::Stream {
                    stream_id,
                    code: ErrorCode::ProtocolError,
                },
                ErrorCode::ProtocolError,
            );
        }
        if let Err(error) = actor.state.transition(StreamEvent::RecvHeaders {
            end_stream: h.end_stream,
        }) {
            return self.reset_stream(
                stream_id,
                map_state_err(stream_id, error),
                ErrorCode::StreamClosed,
            );
        }
        if matches!(actor.response, ResponseState::Initial(_)) {
            return Ok(());
        }
        actor.status = status;
        actor.resp_headers = headers;
        actor.declared_len = length;
        let status = actor.status;
        actor.response_tx = actor.response_tx.take().map(|sink| sink.settle(status));
        if matches!(actor.status.as_u16(), 204 | 304) {
            actor.drop_body = true;
        }
        actor.reserve_body(self.config.max_response_body_bytes);
        if matches!(actor.response_tx, Some(ResponseSink::StreamingEx { .. })) {
            actor.deliver_headers_streaming();
        }
        if h.end_stream {
            self.finish_remote(stream_id)?;
        }
        Ok(())
    }

    fn on_trailers(
        &mut self,
        h: &HeadersFrame,
        trailers: HeaderMap,
        malformed: bool,
    ) -> Result<(), H2Error> {
        let stream_id = h.stream_id;
        if !h.end_stream || malformed {
            return self.reset_stream(
                stream_id,
                H2Error::Stream {
                    stream_id,
                    code: ErrorCode::ProtocolError,
                },
                ErrorCode::ProtocolError,
            );
        }
        let Some(actor) = self.streams.get_mut(stream_id) else {
            return Ok(());
        };
        if actor.response.trailers().is_err() {
            return self.reset_stream(
                stream_id,
                H2Error::Stream {
                    stream_id,
                    code: ErrorCode::ProtocolError,
                },
                ErrorCode::ProtocolError,
            );
        }
        if let Err(error) = actor.state.transition(StreamEvent::RecvTrailers) {
            return self.reset_stream(
                stream_id,
                map_state_err(stream_id, error),
                ErrorCode::StreamClosed,
            );
        }
        actor.trailers = trailers;
        self.finish_remote(stream_id)
    }
}
