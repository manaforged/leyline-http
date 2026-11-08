use bytes::Bytes;
use http::HeaderMap;

use crate::deadline::{Elapsed, within};
use crate::wire::{BlockReader, ResponseBlock};

use super::*;

const BODY_RESERVE_CAP: usize = 64 * 1024;

impl StreamActor {
    fn reserve_body(&mut self) {
        let cap = self.body_limit.room(0);
        if !self.has_body {
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
        let mut block = BlockReader::new(response);
        self.decoder
            .decode_block(&full_fragment, |name, value, typed| {
                block.field(name, value, || typed);
            })
            .map_err(H2Error::Hpack)?;
        self.on_block(&h, block)
    }

    fn on_block(&mut self, h: &HeadersFrame, block: BlockReader) -> Result<(), H2Error> {
        let Some(actor) = self.streams.get_mut(h.stream_id) else {
            return Ok(());
        };
        match block.finish(&mut actor.response) {
            Ok(ResponseBlock::Informational) if !h.end_stream => self.respond(h, None),
            Ok(ResponseBlock::Final {
                status,
                headers,
                length,
            }) => self.respond(h, Some((status, headers, length))),
            Ok(ResponseBlock::Trailers(trailers)) if h.end_stream => self.on_trailers(h, trailers),
            _ => self.reset_protocol(h.stream_id),
        }
    }

    async fn reassemble(&mut self, h: &HeadersFrame) -> Result<Bytes, H2Error> {
        let max_header_block = self.decoder.max_header_list_size();
        let oversized = || H2Error::Connection {
            code: ErrorCode::CompressionError,
            reason: format!("header block exceeds the header list limit ({max_header_block})"),
        };
        if h.fragment.len() > max_header_block {
            return Err(oversized());
        }
        if h.end_headers {
            return Ok(h.fragment.clone());
        }
        let reassembly_timeout = self.config.limits().header_block_reassembly_timeout;
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
        if let Err(error) = actor.state.transition(StreamEvent::RecvHeaders {
            end_stream: h.end_stream,
        }) {
            return self.reset_stream(
                stream_id,
                map_state_err(stream_id, error),
                ErrorCode::StreamClosed,
            );
        }
        let Some((status, headers, length)) = head else {
            return Ok(());
        };
        actor.status = status;
        actor.resp_headers = headers;
        actor.declared_len = length;
        let status = actor.status;
        actor.response_tx = actor.response_tx.take().map(|sink| sink.settle(status));
        actor.has_body = crate::wire::has_body(actor.head_request, status.as_u16());
        actor.reserve_body();
        if matches!(actor.response_tx, Some(ResponseSink::StreamingEx { .. })) {
            actor.deliver_headers_streaming();
        }
        if h.end_stream {
            self.finish_remote(stream_id)?;
        }
        Ok(())
    }

    fn on_trailers(&mut self, h: &HeadersFrame, trailers: HeaderMap) -> Result<(), H2Error> {
        let stream_id = h.stream_id;
        let Some(actor) = self.streams.get_mut(stream_id) else {
            return Ok(());
        };
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
