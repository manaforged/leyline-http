use bytes::Bytes;

use crate::core::deadline::within;
use crate::header_str::HeaderStr;

use super::*;

fn pseudo(actor: &mut StreamActor, decoded: Vec<(Bytes, Bytes)>) -> Option<u16> {
    let mut status = None;
    let mut saw_regular = false;
    let mut bad_status = false;
    for (name, value) in decoded {
        if name.starts_with(b":") {
            let digits = value.as_ref();
            if saw_regular
                || name.as_ref() != b":status"
                || status.is_some()
                || digits.len() != 3
                || !digits.iter().all(u8::is_ascii_digit)
            {
                bad_status = true;
                break;
            }
            status = Some(
                digits
                    .iter()
                    .fold(0_u16, |code, digit| code * 10 + u16::from(*digit - b'0')),
            );
        } else {
            saw_regular = true;
            actor.resp_headers.push((
                HeaderStr::from_bytes_lossy(name),
                HeaderStr::from_bytes_lossy(value),
            ));
        }
    }
    status.filter(|status| !bad_status && *status != 101)
}

const BODY_RESERVE_CAP: usize = 64 * 1024;

fn reserve_body(actor: &mut StreamActor, cap: usize) {
    if actor.drop_body {
        return;
    }
    actor.declared_len = actor
        .resp_headers
        .iter()
        .find(|(name, _)| name.as_str() == "content-length")
        .and_then(|(_, value)| value.as_str().parse::<u64>().ok());
    if matches!(actor.response_tx, Some(ResponseSink::StreamingEx { .. })) {
        return;
    }
    if let Some(len) = actor.declared_len {
        let len = usize::try_from(len).unwrap_or(usize::MAX);
        actor.body.reserve(len.min(cap).min(BODY_RESERVE_CAP));
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) async fn on_headers(&mut self, h: HeadersFrame) -> Result<(), H2Error> {
        let full_fragment = self.reassemble(&h).await?;

        let decoded: Vec<(Bytes, Bytes)> = self
            .decoder
            .decode_header_block(&full_fragment)
            .map_err(H2Error::Hpack)?
            .into_iter()
            .map(|header| (header.name, header.value))
            .collect();

        let first = match self.streams.get(h.stream_id) {
            Some(actor) => !actor.got_headers,
            None => return Ok(()),
        };
        if first {
            self.respond(&h, decoded).await
        } else {
            self.on_trailers(&h, decoded)
        }
    }

    async fn reassemble(&mut self, h: &HeadersFrame) -> Result<Bytes, H2Error> {
        if h.end_headers {
            return Ok(h.fragment.clone());
        }
        if self.writer.pending() > 0 {
            self.writer.flush().await?;
        }
        let max_header_block = self.config.max_header_block_bytes;
        let reassembly_timeout = self.config.header_block_reassembly_timeout;
        let started = tokio::time::Instant::now();
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
        let next = within(Some(remaining), self.reader.next())
            .await
            .map_err(|_| H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: format!("CONTINUATION reassembly exceeded {limit:?}"),
            })??;
        next.ok_or_else(|| H2Error::Connection {
            code: ErrorCode::ProtocolError,
            reason: "connection closed during CONTINUATION".into(),
        })
    }

    async fn respond(
        &mut self,
        h: &HeadersFrame,
        decoded: Vec<(Bytes, Bytes)>,
    ) -> Result<(), H2Error> {
        let stream_id = h.stream_id;
        let Some(actor) = self.streams.get_mut(stream_id) else {
            return Ok(());
        };
        if let Err(e) = actor.state.transition(StreamEvent::RecvHeaders {
            end_stream: h.end_stream,
        }) {
            let err = map_state_err(stream_id, e);
            self.fail_stream(stream_id, err);
            let _ = self
                .writer
                .write_rst_stream(stream_id, ErrorCode::StreamClosed)
                .await;
            return Ok(());
        }
        let Some(status) = pseudo(actor, decoded) else {
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
        reserve_body(actor, self.config.max_response_body_bytes);
        if matches!(actor.response_tx, Some(ResponseSink::StreamingEx { .. })) {
            actor.deliver_headers_streaming();
        }
        if h.end_stream {
            self.finish_remote(stream_id).await?;
        }
        Ok(())
    }

    fn on_trailers(
        &mut self,
        h: &HeadersFrame,
        decoded: Vec<(Bytes, Bytes)>,
    ) -> Result<(), H2Error> {
        let stream_id = h.stream_id;
        if !h.end_stream || decoded.iter().any(|(name, _)| name.starts_with(b":")) {
            self.fail_stream(
                stream_id,
                H2Error::Stream {
                    stream_id,
                    code: ErrorCode::ProtocolError,
                },
            );
            return Ok(());
        }
        let Some(actor) = self.streams.get_mut(stream_id) else {
            return Ok(());
        };
        if let Err(e) = actor.state.transition(StreamEvent::RecvTrailers) {
            let err = map_state_err(stream_id, e);
            self.fail_stream(stream_id, err);
            return Ok(());
        }
        let mut th = Vec::new();
        for (name, value) in decoded {
            th.push((
                HeaderStr::from_bytes_lossy(name),
                HeaderStr::from_bytes_lossy(value),
            ));
        }
        actor.trailers = Some(th);
        self.complete_stream(stream_id);
        Ok(())
    }
}
