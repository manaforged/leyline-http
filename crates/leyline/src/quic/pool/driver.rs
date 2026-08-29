//! The h3 connection driver task: sole owner of the QUIC connection.
//! Owns cooperative multiplexing of request streams, body pumping, and
//! event draining for the pooled `H3Client` handle.
use super::*;

impl H3Driver {
    pub(super) async fn run(self) {
        let EstablishedH3 {
            socket,
            mut conn,
            mut h3,
            peer_addr,
            local_addr,
            max_udp_payload,
            max_response_body_bytes,
            tls: _,
        } = self.established;
        let mut command_rx = self.command_rx;
        let body_chunk_tx = self.body_chunk_tx;
        let mut body_chunk_rx = self.body_chunk_rx;
        let closed = self.closed;
        let mut streams = self.streams;

        let mut out = vec![0u8; max_udp_payload];
        let mut buf = vec![0u8; 65_535];
        let mut pending: VecDeque<H3Command> = VecDeque::new();
        let mut commands_closed = false;
        // Set when the server GOAWAYs: the connection is draining. New
        // requests fail as not-sent (provable: never queued to the wire)
        // so the pool retries them on a fresh connection; in-flight
        // streams run to completion.
        let mut draining = false;
        let mut admit_cap: Option<usize> = None;

        loop {
            // Reap any stream whose caller dropped its receiver (an outer timeout
            // fired, or the request was cancelled) before doing per-stream work,
            // so a freed QUIC stream-credit slot is available to a request started
            // in this same iteration.
            sweep_cancelled_streams(&mut conn, &mut streams);

            // Start any queued requests now that the connection can take them,
            // (re)attempt flow-control-parked request bodies, then push any
            // ready streaming-response body into its consumer channel.
            start_pending(
                &mut h3,
                &mut conn,
                &mut streams,
                &mut pending,
                &body_chunk_tx,
                admit_cap,
            );
            write_pending_request_bodies(&mut h3, &mut conn, &mut streams);
            let stream_backpressured = pump_streaming_bodies(
                &mut h3,
                &mut conn,
                &mut streams,
                &mut buf,
                max_response_body_bytes,
            );

            if let Err(e) = flush_egress(&socket, &mut conn, &mut out).await {
                fail_all(&mut streams, &mut pending, &closed, e);
                return;
            }

            if conn.is_closed() {
                let reason = close_reason("h3", 0, &conn);
                fail_all(&mut streams, &mut pending, &closed, reason);
                return;
            }

            // Graceful close: all handles dropped, nothing left in flight.
            if commands_closed && streams.is_empty() && pending.is_empty() {
                let _ = conn.close(true, 0x100, b"done");
                let _ = flush_egress(&socket, &mut conn, &mut out).await;
                closed.store(true, Ordering::Release);
                return;
            }

            let mut timeout = conn.timeout().unwrap_or(Duration::from_secs(5));
            // While a streaming response is back-pressured, nothing wakes the
            // driver when the consumer drains the full channel — cap the wait so
            // the pump retries promptly instead of stalling to the idle timeout.
            if stream_backpressured {
                timeout = timeout.min(STREAM_PUMP_INTERVAL);
            }
            // While any stream is in flight, cap the wait so a caller that drops
            // its receiver on an otherwise-idle connection is reaped by the next
            // `sweep_cancelled_streams` within a bounded window, rather than
            // holding stream credit until the QUIC idle timeout.
            if !streams.is_empty() {
                timeout = timeout.min(CANCEL_SWEEP_INTERVAL);
            }

            tokio::select! {
                cmd = command_rx.recv(), if !commands_closed => match cmd {
                    Some(cmd) => {
                        if draining {
                            let H3Command::Request { resp_tx, .. } = cmd;
                            let _ = resp_tx
                                .send(Err("server sent GOAWAY: request not sent".into()));
                        } else {
                            pending.push_back(cmd);
                        }
                    }
                    None => commands_closed = true,
                },
                chunk = body_chunk_rx.recv() => {
                    // The driver holds `body_chunk_tx`, so `recv` never yields
                    // `None`; a missing chunk is impossible here.
                    if let Some(chunk) = chunk {
                        on_request_body_chunk(&mut conn, &mut streams, chunk);
                    }
                }
                recv = socket.recv(&mut buf) => match recv {
                    Ok(len) => {
                        let recv_info = quiche::RecvInfo { from: peer_addr, to: local_addr };
                        if let Err(e) = conn.recv(&mut buf[..len], recv_info) {
                            fail_all(&mut streams, &mut pending, &closed, format!("quic recv: {e}"));
                            return;
                        }
                        match drain_h3_events(
                            &mut h3,
                            &mut conn,
                            &mut streams,
                            &mut pending,
                            &mut buf,
                            max_response_body_bytes,
                            &mut admit_cap,
                        ) {
                            Err(e) => {
                                fail_all(&mut streams, &mut pending, &closed, e);
                                return;
                            }
                            Ok(true) => {
                                draining = true;
                                closed.store(true, Ordering::Release);
                                for cmd in pending.drain(..) {
                                    let H3Command::Request { resp_tx, .. } = cmd;
                                    let _ = resp_tx.send(Err(
                                        "server sent GOAWAY: request not sent".into()
                                    ));
                                }
                                tracing::debug!(
                                    target: "leyline::quic",
                                    "h3 server GOAWAY: connection draining, pool handle closed"
                                );
                            }
                            Ok(false) => {}
                        }
                    }
                    Err(e) => {
                        fail_all(&mut streams, &mut pending, &closed, format!("udp recv: {e}"));
                        return;
                    }
                },
                _ = tokio::time::sleep(timeout) => conn.on_timeout(),
            }
        }
    }
}

/// Open request streams for queued commands. Stops (leaving the rest queued)
/// the moment the connection won't accept another stream, and retries on the
/// next loop iteration once a MAX_STREAMS update arrives.
pub(super) fn start_pending(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
    pending: &mut VecDeque<H3Command>,
    body_chunk_tx: &mpsc::Sender<H3BodyChunk>,
    admit_cap: Option<usize>,
) {
    while let Some(H3Command::Request {
        headers,
        body,
        body_stream,
        ..
    }) = pending.front()
    {
        // The server's h3 MAX_CONCURRENT_STREAMS budget: once it has
        // refused a stream, queue instead of triggering another refusal.
        // In-flight count falls as streams finish, so queued requests
        // drain naturally.
        if admit_cap.is_some_and(|cap| streams.len() >= cap) {
            break;
        }
        let streaming = body_stream.is_some();
        // FIN rides HEADERS only with no body at all; a buffered body finishes
        // on its last DATA, and a streaming body on its EOF — never here.
        let fin = !streaming && body.as_ref().is_none_or(Bytes::is_empty);
        // Retry stash for a later REQUEST_REJECTED re-queue (buffered only).
        let retry = (!streaming).then(|| (headers.clone(), body.clone(), 0u8));
        match h3.send_request(conn, headers, fin) {
            Ok(stream_id) => {
                let Some(H3Command::Request {
                    body,
                    body_stream,
                    stream_body_tx,
                    resp_tx,
                    ..
                }) = pending.pop_front()
                else {
                    unreachable!("front matched Request above");
                };
                let mut stream = H3Stream::new(resp_tx, body, stream_body_tx, streaming);
                stream.retry = retry;
                // A streaming body's chunks arrive on a pump task that tags them
                // with this now-known stream id and relays them to the driver.
                // The driver keeps the pump's abort handle (so teardown can
                // cancel it) and its byte-credit (so it can grant back-pressure).
                if let Some(body_stream) = body_stream {
                    let credit = Arc::new(Semaphore::new(UPLOAD_WINDOW));
                    let pump = tokio::spawn(pump_request_body(
                        stream_id,
                        body_stream,
                        body_chunk_tx.clone(),
                        Arc::clone(&credit),
                    ));
                    stream.upload_credit = Some(credit);
                    stream.pump = Some(pump.abort_handle());
                }
                write_request_body(h3, conn, stream_id, &mut stream);
                streams.insert(stream_id, stream);
            }
            // Stream limit reached — retry after the next MAX_STREAMS update.
            Err(quiche::h3::Error::StreamBlocked) | Err(quiche::h3::Error::Done) => break,
            Err(e) => {
                if let Some(H3Command::Request { resp_tx, .. }) = pending.pop_front() {
                    let _ = resp_tx.send(Err(format!("h3 send_request: {e}")));
                }
            }
        }
    }
}

/// Write any queued request-body bytes — chunks that flow control parked
/// mid-write, plus chunks freshly relayed from a streaming body's pump.
pub(super) fn write_pending_request_bodies(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
) {
    for (stream_id, stream) in streams.iter_mut() {
        if stream.body_write_pending() {
            write_request_body(h3, conn, *stream_id, stream);
        }
    }
}

/// Write as much of a stream's queued request body as flow control allows.
///
/// The terminating FIN rides the final bytes only once the body is complete
/// (`body_eof`) and this is the last queued chunk — so a streaming body never
/// finishes early on an interior chunk. quiche applies the FIN only when the
/// whole buffer is flushed, so a partial write simply retries next loop. If the
/// body completed but the queue is already empty (an empty streaming body, or a
/// final chunk written before EOF was known), an explicit empty-FIN write
/// closes the send side.
pub(super) fn write_request_body(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    stream_id: u64,
    stream: &mut H3Stream,
) {
    while let Some(front) = stream.out_chunks.front() {
        let remaining = &front[stream.out_offset..];
        let last_chunk = stream.body_eof && stream.out_chunks.len() == 1;
        match h3.send_body(conn, stream_id, remaining, last_chunk) {
            Ok(0) => return, // flow control parked; retry next loop
            Ok(written) => {
                stream.out_offset += written;
                // Return byte-credit for bytes now on the wire so the pump may
                // relay more (streaming bodies only; buffered have no credit).
                if let Some(credit) = &stream.upload_credit {
                    credit.add_permits(written);
                }
                if stream.out_offset >= front.len() {
                    stream.out_chunks.pop_front();
                    stream.out_offset = 0;
                    if last_chunk {
                        stream.fin_sent = true;
                    }
                }
            }
            Err(quiche::h3::Error::Done) | Err(quiche::h3::Error::StreamBlocked) => return,
            Err(e) => {
                stream.deliver(Err(format!("h3 send_body: {e}")));
                // Leave the dead stream in the map; the peer Reset / connection
                // teardown removes it. Stop trying to write it.
                stream.out_chunks.clear();
                stream.out_offset = 0;
                stream.fin_sent = true;
                return;
            }
        }
    }

    // Queue drained but the completed body's FIN hasn't gone out yet.
    if stream.body_eof && !stream.fin_sent {
        match h3.send_body(conn, stream_id, &[], true) {
            Ok(_) => stream.fin_sent = true,
            Err(quiche::h3::Error::Done) | Err(quiche::h3::Error::StreamBlocked) => {}
            Err(e) => {
                stream.deliver(Err(format!("h3 send_body fin: {e}")));
                stream.fin_sent = true;
            }
        }
    }
}

/// Read a streaming request body and relay each chunk to the driver tagged with
/// `stream_id`. Runs on its own task so the driver's single-owner invariant
/// holds — the body `Stream`'s `.await` never blocks the connection loop.
async fn pump_request_body(
    stream_id: u64,
    mut body: H3RequestBodyStream,
    tx: mpsc::Sender<H3BodyChunk>,
    credit: Arc<Semaphore>,
) {
    use futures_util::StreamExt;
    while let Some(item) = body.next().await {
        match item {
            Ok(mut data) => {
                // Relay in <= UPLOAD_CHUNK slices, acquiring byte-credit before
                // each. Credit caps in-flight upload bytes at UPLOAD_WINDOW; the
                // driver returns it as bytes reach the wire, so a flow-control-
                // stalled peer blocks this acquire and back-pressures the source.
                while !data.is_empty() {
                    let take = data.len().min(UPLOAD_CHUNK);
                    let slice = data.split_to(take);
                    let Ok(permit) = credit.acquire_many(take as u32).await else {
                        return; // stream torn down
                    };
                    permit.forget(); // returned by the driver via add_permits
                    if tx
                        .send(H3BodyChunk::Chunk {
                            stream_id,
                            data: slice,
                        })
                        .await
                        .is_err()
                    {
                        return; // driver gone
                    }
                }
            }
            Err(error) => {
                let _ = tx
                    .send(H3BodyChunk::Eof {
                        stream_id,
                        error: Some(error),
                    })
                    .await;
                return;
            }
        }
    }
    let _ = tx
        .send(H3BodyChunk::Eof {
            stream_id,
            error: None,
        })
        .await;
}

/// Apply a relayed request-body chunk to its stream. Appends bytes (the loop
/// flushes them via [`write_pending_request_bodies`]), or on EOF either marks
/// the body complete (FIN now allowed) or, on a source error, resets the send
/// side and fails the request.
pub(super) fn on_request_body_chunk(
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
    chunk: H3BodyChunk,
) {
    match chunk {
        H3BodyChunk::Chunk { stream_id, data } => {
            // A chunk for a stream that's already gone (reset/finished) is
            // dropped; its pump self-terminates when the body ends.
            if let Some(stream) = streams.get_mut(&stream_id) {
                stream.out_chunks.push_back(data);
            }
        }
        H3BodyChunk::Eof { stream_id, error } => {
            let Some(stream) = streams.get_mut(&stream_id) else {
                return;
            };
            match error {
                None => stream.body_eof = true,
                Some(e) => {
                    // The body source failed mid-upload (the pump self-terminated
                    // by sending this error Eof). Reset both halves — RESET_STREAM
                    // our send side, STOP_SENDING the response we'll never read —
                    // and fail the request.
                    let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Write, 0);
                    let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Read, 0);
                    let msg = format!("h3 request body stream error: {e}");
                    if stream.head_sent {
                        if let Some(tx) = &stream.stream_tx {
                            deliver_stream_error(tx, std::io::Error::other(msg));
                        }
                    } else {
                        stream.deliver(Err(msg));
                    }
                    streams.remove(&stream_id);
                }
            }
        }
    }
}

/// Reset the request-upload (write) half of a stream and cancel its pump. Used
/// when a stream is torn down for a read-side reason — the peer responded and
/// finished early, or the response consumer dropped its receiver — while an
/// upload is still in flight, so the peer sees a RESET_STREAM rather than a
/// silently abandoned half-open send side, and the pump stops at once.
fn reset_upload_half(conn: &mut quiche::Connection, stream_id: u64, stream: &mut H3Stream) {
    if stream.send_side_open() {
        let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Write, 0);
    }
    stream.cancel_upload();
}

/// True when the caller has abandoned this stream: it dropped the response
/// oneshot before the head was delivered (a buffered request, or a streaming one
/// pre-head), or — once the head has been streamed — dropped the body-channel
/// receiver. Either is the signal that an outer timeout (`response_header` /
/// `total`) or an explicit cancellation fired and the stream should be torn down.
pub(super) fn stream_is_cancelled(stream: &H3Stream) -> bool {
    match stream.resp_tx.as_ref() {
        Some(tx) => tx.is_closed(),
        None => stream
            .stream_tx
            .as_ref()
            .map(|tx| tx.is_closed())
            .unwrap_or(false),
    }
}

/// Ids of streams whose caller has dropped its receiver. Split out from
/// [`sweep_cancelled_streams`] so the selection logic is unit-testable without a
/// live `quiche::Connection`.
pub(super) fn cancelled_stream_ids(streams: &HashMap<u64, H3Stream>) -> Vec<u64> {
    streams
        .iter()
        .filter(|(_, s)| stream_is_cancelled(s))
        .map(|(&id, _)| id)
        .collect()
}

/// Reap streams whose caller dropped its receiver, freeing the QUIC stream-credit
/// slot instead of letting an orphan linger until the connection's idle timeout.
///
/// Without this, an outer timeout firing before the peer replies — the
/// silent-proxy case `response_header` exists to catch — leaves the stream in the
/// map with no peer event to remove it, holding `max_concurrent_bidi_streams`
/// credit and flow-control window. STOP_SENDING (`Shutdown::Read`) abandons the
/// response we will never read; [`reset_upload_half`] RESET_STREAMs the send half
/// if the upload is still open and stops the body pump. The H2 driver's
/// `sweep_cancelled_streams` is the counterpart this mirrors.
pub(super) fn sweep_cancelled_streams(
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
) {
    for id in cancelled_stream_ids(streams) {
        if let Some(mut stream) = streams.remove(&id) {
            let _ = conn.stream_shutdown(id, quiche::Shutdown::Read, 0);
            reset_upload_half(conn, id, &mut stream);
        }
    }
}

/// Drain all ready HTTP/3 events, dispatching each to its request stream.
/// Returns `Err` only on a connection-fatal HTTP/3 error.
/// Pumps h3 events. Returns `true` when the server sent GOAWAY: the
/// connection is draining and must not admit new requests.
pub(super) fn drain_h3_events(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
    pending: &mut VecDeque<H3Command>,
    scratch: &mut [u8],
    max_response_body_bytes: u64,
    admit_cap: &mut Option<usize>,
) -> Result<bool, String> {
    loop {
        match h3.poll(conn) {
            Ok((stream_id, quiche::h3::Event::Headers { list, .. })) => {
                let list = list
                    .iter()
                    .map(|header| {
                        (
                            String::from_utf8_lossy(header.name()).to_string(),
                            String::from_utf8_lossy(header.value()).to_string(),
                        )
                    })
                    .collect::<Vec<_>>();
                let Some(stream) = streams.get_mut(&stream_id) else {
                    continue;
                };
                if let Err(message) = stream.headers(&list) {
                    let _ = conn.stream_shutdown(
                        stream_id,
                        quiche::Shutdown::Read,
                        quiche::h3::WireErrorCode::MessageError as u64,
                    );
                    let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Write, 0);
                    stream.deliver_error(message.into());
                    streams.remove(&stream_id);
                    continue;
                }
                // Streaming: hand the caller the head as soon as it arrives;
                // body chunks then flow through the channel via the pump.
                if stream.is_streaming()
                    && stream.response == H3ResponseState::Final
                    && !stream.head_sent
                {
                    stream.deliver_head();
                }
            }
            Ok((stream_id, quiche::h3::Event::Data)) => {
                let Some(stream) = streams.get_mut(&stream_id) else {
                    // Drain quiche's buffer for an unknown stream so it does
                    // not wedge, but discard the bytes.
                    while let Ok(n) = h3.recv_body(conn, stream_id, scratch) {
                        if n == 0 {
                            break;
                        }
                    }
                    continue;
                };
                if let Err(message) = stream.data() {
                    let _ = conn.stream_shutdown(
                        stream_id,
                        quiche::Shutdown::Read,
                        quiche::h3::WireErrorCode::MessageError as u64,
                    );
                    let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Write, 0);
                    stream.deliver_error(message.into());
                    streams.remove(&stream_id);
                    continue;
                }
                // Streaming: drain inline, matching the buffered path's
                // `recv_body` timing so flow-control credit is granted in
                // immediate response to this packet. `forward_stream_body`
                // applies channel back-pressure (stashing one chunk and leaving
                // the rest in quiche so QUIC flow control throttles the origin).
                if stream.is_streaming() {
                    if forward_stream_body(
                        h3,
                        conn,
                        stream_id,
                        stream,
                        scratch,
                        max_response_body_bytes,
                    ) {
                        streams.remove(&stream_id);
                    }
                    continue;
                }
                while let Ok(n) = h3.recv_body(conn, stream_id, scratch) {
                    if n == 0 {
                        break;
                    }
                    // Bound the buffered body — an unbounded QUIC flow-control
                    // window otherwise lets a malicious origin OOM the client.
                    // Note: per-stream cap; aggregate across multiplexed
                    // streams is bounded by caller concurrency (the origin
                    // can't open client-initiated request streams), matching the
                    // H2 path. Add a connection-wide budget if a single host's
                    // concurrent responses need a tighter ceiling.
                    if let Err(new_len) =
                        check_body_budget(stream.body_bytes_seen, n, max_response_body_bytes)
                    {
                        let _ = conn.stream_shutdown(
                            stream_id,
                            quiche::Shutdown::Read,
                            quiche::h3::WireErrorCode::ExcessiveLoad as u64,
                        );
                        stream.deliver(Err(format!(
                            "h3: response body exceeded max_response_body_bytes ({new_len} > {max_response_body_bytes})"
                        )));
                        streams.remove(&stream_id);
                        break;
                    }
                    stream.body_bytes_seen += n;
                    stream.body.extend_from_slice(&scratch[..n]);
                }
            }
            Ok((stream_id, quiche::h3::Event::Finished)) => {
                let invalid = streams
                    .get(&stream_id)
                    .and_then(|stream| stream.finish().err());
                if let Some(message) = invalid {
                    let _ = conn.stream_shutdown(
                        stream_id,
                        quiche::Shutdown::Read,
                        quiche::h3::WireErrorCode::MessageError as u64,
                    );
                    let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Write, 0);
                    if let Some(mut stream) = streams.remove(&stream_id) {
                        stream.deliver_error(message.into());
                    }
                    continue;
                }
                let streaming = streams.get(&stream_id).map(H3Stream::is_streaming);
                match streaming {
                    // Streaming: mark finished; the pump drains the remaining
                    // body, then closes the channel (EOF) and removes the stream.
                    Some(true) => {
                        if let Some(stream) = streams.get_mut(&stream_id) {
                            // Peer responded before we finished uploading our
                            // request body (an early 4xx/413) — RESET our send
                            // half and cancel the pump so it stops producing into
                            // a stream we'll never finish, then keep draining the
                            // response body that's still arriving.
                            reset_upload_half(conn, stream_id, stream);
                            stream.peer_finished = true;
                        }
                    }
                    // Buffered: deliver the whole response now.
                    Some(false) => {
                        if let Some(mut stream) = streams.remove(&stream_id) {
                            // Peer responded before we finished uploading (an
                            // early 4xx/413) — abort our send side so the
                            // half-open stream doesn't leak QUIC stream credit.
                            if stream.send_side_open() {
                                let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Write, 0);
                            }
                            let resp = H3Response {
                                status: stream.status,
                                headers: std::mem::take(&mut stream.headers),
                                body: std::mem::take(&mut stream.body),
                            };
                            stream.deliver(Ok(resp));
                        }
                    }
                    None => {}
                }
            }
            Ok((stream_id, quiche::h3::Event::Reset(e))) => {
                // H3_REQUEST_REJECTED (0x10b): the server refused this
                // stream — its MAX_CONCURRENT_STREAMS budget was full when
                // we opened. Cap admission at the current in-flight count.
                if e == 0x10b {
                    // Converge: halve the admission cap toward the server's
                    // real budget instead of re-triggering refusals at the
                    // same burst size.
                    let cur = streams.len().max(1);
                    let next = match *admit_cap {
                        Some(c) => c.min(cur / 2).max(1),
                        None => (cur / 2).max(1),
                    };
                    *admit_cap = Some(next);
                }
                if let Some(mut stream) = streams.remove(&stream_id) {
                    // RFC 9114 8.1: REQUEST_REJECTED means the request was
                    // not processed — re-queue unconditionally (buffered
                    // requests only). The caller's overall timeout bounds
                    // the wait; the admission cap stops new refusals.
                    if e == 0x10b && !stream.head_sent {
                        if let Some((headers, body, _)) = stream.retry.take() {
                            pending.push_back(H3Command::Request {
                                headers,
                                body,
                                body_stream: None,
                                stream_body_tx: None,
                                resp_tx: stream.resp_tx.take().expect("buffered keeps resp_tx"),
                            });
                            continue;
                        }
                    }
                }
                if let Some(mut stream) = streams.remove(&stream_id) {
                    let msg = format!("h3 stream reset: {e}");
                    if stream.head_sent {
                        // Streaming head already delivered — surface the error
                        // through the body channel as a final item (reliable
                        // delivery, so a full channel doesn't drop it into a
                        // silent EOF).
                        if let Some(tx) = &stream.stream_tx {
                            deliver_stream_error(tx, std::io::Error::other(msg));
                        }
                    } else {
                        stream.deliver(Err(msg));
                    }
                }
            }
            Ok((_, quiche::h3::Event::PriorityUpdate)) => {}
            Ok((_, quiche::h3::Event::GoAway)) => return Ok(true),
            Err(quiche::h3::Error::Done) => return Ok(false),
            Err(e) => return Err(format!("h3 poll: {e}")),
        }
    }
}

/// Drain ready body bytes for one streaming response into its bounded channel,
/// applying back-pressure: a chunk the channel can't accept yet is stashed
/// (`stalled`) and reading stops immediately, leaving the rest in quiche so
/// QUIC flow control throttles the origin. Returns `true` when the stream is
/// finished and should be removed (EOF channel-close, body-cap hit, or the
/// consumer dropped its receiver).
///
/// Called inline from the `Data` event — matching the buffered path's
/// `recv_body` timing so flow-control credit is granted in immediate response
/// to each packet — and again from `pump_streaming_bodies` to retry a stalled
/// chunk and emit EOF once the peer has finished.
pub(super) fn forward_stream_body(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    stream_id: u64,
    stream: &mut H3Stream,
    scratch: &mut [u8],
    max_response_body_bytes: u64,
) -> bool {
    use tokio::sync::mpsc::error::TrySendError;

    let Some(tx) = stream.stream_tx.clone() else {
        return false; // buffered stream
    };

    // 1. Retry a back-pressure-stalled chunk before reading more.
    if let Some(chunk) = stream.stalled.take() {
        match tx.try_send(Ok(chunk)) {
            Ok(()) => {}
            Err(TrySendError::Full(item)) => {
                if let Ok(b) = item {
                    stream.stalled = Some(b);
                }
                return false; // still full; try again next loop
            }
            Err(TrySendError::Closed(_)) => {
                // Response consumer dropped its receiver — STOP_SENDING the
                // response and RESET any still-active upload, then remove.
                let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Read, 0);
                reset_upload_half(conn, stream_id, stream);
                return true;
            }
        }
    }

    // 2. Drain quiche into the channel until it's full or empty.
    let mut drained_clean = false;
    loop {
        match h3.recv_body(conn, stream_id, scratch) {
            Ok(0) => {
                drained_clean = true;
                break;
            }
            Ok(n) => {
                if let Err(new_len) =
                    check_body_budget(stream.body_bytes_seen, n, max_response_body_bytes)
                {
                    let _ = conn.stream_shutdown(
                        stream_id,
                        quiche::Shutdown::Read,
                        quiche::h3::WireErrorCode::ExcessiveLoad as u64,
                    );
                    deliver_stream_error(
                        &tx,
                        std::io::Error::other(format!(
                            "h3: response body exceeded max_response_body_bytes ({new_len} > {max_response_body_bytes})"
                        )),
                    );
                    return true;
                }
                stream.body_bytes_seen += n;
                let chunk = Bytes::copy_from_slice(&scratch[..n]);
                match tx.try_send(Ok(chunk)) {
                    Ok(()) => {}
                    Err(TrySendError::Full(item)) => {
                        if let Ok(b) = item {
                            stream.stalled = Some(b);
                        }
                        break; // back-pressure: stop draining this stream
                    }
                    Err(TrySendError::Closed(_)) => {
                        // Response consumer dropped its receiver — STOP_SENDING
                        // the response and RESET any still-active upload.
                        let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Read, 0);
                        reset_upload_half(conn, stream_id, stream);
                        return true;
                    }
                }
            }
            Err(quiche::h3::Error::Done) => {
                drained_clean = true;
                break;
            }
            Err(e) => {
                deliver_stream_error(&tx, std::io::Error::other(format!("h3 recv_body: {e}")));
                return true;
            }
        }
    }

    // 3. Fully drained and the peer finished → close the channel (EOF). Check
    //    the transport FIN directly, not only the H3 `Finished` event, which
    //    needs a post-drain `poll()` that never runs when the FIN rode the last
    //    packet and a back-pressured tail drained here rather than inline.
    // Note: trailers leave stream_finished false until polled → peer_finished covers them.
    if drained_clean
        && stream.stalled.is_none()
        && (stream.peer_finished || conn.stream_finished(stream_id))
    {
        stream.stream_tx = None; // drop the driver's sender → consumer EOF
        return true;
    }
    false
}

/// Retry a stalled chunk and emit EOF for every streaming response once its
/// peer has finished. The primary body read happens inline in the `Data` event
/// (see [`forward_stream_body`]); this pass exists to make progress when no new
/// packet arrives — a consumer draining a full channel, or the peer's
/// `Finished` landing after the last body was already drained.
///
/// Returns `true` if any streaming stream ended this pass back-pressured (a
/// chunk stashed because its channel is full). The driver caps its next select
/// timeout when so, since nothing else wakes it when the consumer drains a
/// full channel — without this a stalled stream waits for the idle timeout.
pub(super) fn pump_streaming_bodies(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
    scratch: &mut [u8],
    max_response_body_bytes: u64,
) -> bool {
    let mut to_remove: Vec<u64> = Vec::new();
    for (stream_id, stream) in streams.iter_mut() {
        if stream.stream_tx.is_none() {
            continue; // buffered stream
        }
        if forward_stream_body(
            h3,
            conn,
            *stream_id,
            stream,
            scratch,
            max_response_body_bytes,
        ) {
            to_remove.push(*stream_id);
        }
    }

    for id in to_remove {
        streams.remove(&id);
    }

    streams.values().any(|s| s.stalled.is_some())
}

/// Deliver a terminal error to a streaming consumer reliably. A spawned task
/// awaits a free channel slot and appends the error after any already-queued
/// chunks, so a back-pressured consumer (full channel) sees the error instead
/// of the silent, truncating EOF that a dropped `try_send` leaves once the
/// driver drops the sender. If the consumer already dropped its receiver, the
/// send fails fast and the task exits.
pub(super) fn deliver_stream_error(tx: &mpsc::Sender<std::io::Result<Bytes>>, err: std::io::Error) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let _ = tx.send(Err(err)).await;
    });
}

/// Fail every in-flight and queued request with `reason` and mark the
/// connection closed. Called on any connection-fatal path.
pub(super) fn fail_all(
    streams: &mut HashMap<u64, H3Stream>,
    pending: &mut VecDeque<H3Command>,
    closed: &AtomicBool,
    reason: String,
) {
    closed.store(true, Ordering::Release);
    for (_, mut stream) in streams.drain() {
        if stream.head_sent {
            // Streaming, head already delivered — push the failure into the
            // body channel so the consumer sees an error, not a silent EOF that
            // would look like a complete (but truncated) body. Reliable delivery
            // (not try_send) so a back-pressured consumer still gets the error.
            if let Some(tx) = &stream.stream_tx {
                deliver_stream_error(tx, std::io::Error::other(reason.clone()));
            }
        } else {
            stream.deliver(Err(reason.clone()));
        }
    }
    for cmd in pending.drain(..) {
        let H3Command::Request { resp_tx, .. } = cmd;
        let _ = resp_tx.send(Err(reason.clone()));
    }
}
