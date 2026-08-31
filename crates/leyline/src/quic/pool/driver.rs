//! The h3 connection driver task: sole owner of the QUIC connection.
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
        let mut draining = false;
        let mut admit_cap: Option<usize> = None;

        loop {
            sweep_cancelled_streams(&mut conn, &mut streams);

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

            if commands_closed && streams.is_empty() && pending.is_empty() {
                let _ = conn.close(true, 0x100, b"done");
                let _ = flush_egress(&socket, &mut conn, &mut out).await;
                closed.store(true, Ordering::Release);
                return;
            }

            let mut timeout = conn.timeout().unwrap_or(Duration::from_secs(5));
            if stream_backpressured {
                timeout = timeout.min(STREAM_PUMP_INTERVAL);
            }
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

/// Open request streams for queued commands.
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
        if admit_cap.is_some_and(|cap| streams.len() >= cap) {
            break;
        }
        let streaming = body_stream.is_some();
        let fin = !streaming && body.as_ref().is_none_or(Bytes::is_empty);
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
            Err(quiche::h3::Error::StreamBlocked) | Err(quiche::h3::Error::Done) => break,
            Err(e) => {
                if let Some(H3Command::Request { resp_tx, .. }) = pending.pop_front() {
                    let _ = resp_tx.send(Err(format!("h3 send_request: {e}")));
                }
            }
        }
    }
}

/// Write any queued request-body bytes — chunks that flow control parked mid-write, plus chunks freshly relayed from a streaming body's pump.
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
            Ok(0) => return,
            Ok(written) => {
                stream.out_offset += written;
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
                stream.out_chunks.clear();
                stream.out_offset = 0;
                stream.fin_sent = true;
                return;
            }
        }
    }

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

/// Read a streaming request body and relay each chunk to the driver tagged with `stream_id`.
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
                while !data.is_empty() {
                    let take = data.len().min(UPLOAD_CHUNK);
                    let slice = data.split_to(take);
                    let Ok(permit) = credit.acquire_many(take as u32).await else {
                        return;
                    };
                    permit.forget();
                    if tx
                        .send(H3BodyChunk::Chunk {
                            stream_id,
                            data: slice,
                        })
                        .await
                        .is_err()
                    {
                        return;
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

/// Apply a relayed request-body chunk to its stream.
pub(super) fn on_request_body_chunk(
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
    chunk: H3BodyChunk,
) {
    match chunk {
        H3BodyChunk::Chunk { stream_id, data } => {
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

/// Reset the request-upload (write) half of a stream and cancel its pump.
fn reset_upload_half(conn: &mut quiche::Connection, stream_id: u64, stream: &mut H3Stream) {
    if stream.send_side_open() {
        let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Write, 0);
    }
    stream.cancel_upload();
}

/// True when the caller has abandoned this stream: it dropped the response oneshot before the head was delivered (a buffered request, or a streaming one pre-head), or — once the head has been streamed — dropped the body-channel receiver.
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

/// Ids of streams whose caller has dropped its receiver.
pub(super) fn cancelled_stream_ids(streams: &HashMap<u64, H3Stream>) -> Vec<u64> {
    streams
        .iter()
        .filter(|(_, s)| stream_is_cancelled(s))
        .map(|(&id, _)| id)
        .collect()
}

/// Reap streams whose caller dropped its receiver, freeing the QUIC stream-credit slot instead of letting an orphan linger until the connection's idle timeout.
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
                if stream.is_streaming()
                    && stream.response == H3ResponseState::Final
                    && !stream.head_sent
                {
                    stream.deliver_head();
                }
            }
            Ok((stream_id, quiche::h3::Event::Data)) => {
                let Some(stream) = streams.get_mut(&stream_id) else {
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
                    Some(true) => {
                        if let Some(stream) = streams.get_mut(&stream_id) {
                            reset_upload_half(conn, stream_id, stream);
                            stream.peer_finished = true;
                        }
                    }
                    Some(false) => {
                        if let Some(mut stream) = streams.remove(&stream_id) {
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
                if e == 0x10b {
                    let cur = streams.len().max(1);
                    let next = match *admit_cap {
                        Some(c) => c.min(cur / 2).max(1),
                        None => (cur / 2).max(1),
                    };
                    *admit_cap = Some(next);
                }
                if let Some(mut stream) = streams.remove(&stream_id) {
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

/// Drain ready body bytes for one streaming response into its bounded channel, applying back-pressure: a chunk the channel can't accept yet is stashed (`stalled`) and reading stops immediately, leaving the rest in quiche so QUIC flow control throttles the origin.
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
        return false;
    };

    if let Some(chunk) = stream.stalled.take() {
        match tx.try_send(Ok(chunk)) {
            Ok(()) => {}
            Err(TrySendError::Full(item)) => {
                if let Ok(b) = item {
                    stream.stalled = Some(b);
                }
                return false;
            }
            Err(TrySendError::Closed(_)) => {
                let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Read, 0);
                reset_upload_half(conn, stream_id, stream);
                return true;
            }
        }
    }

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
                        break;
                    }
                    Err(TrySendError::Closed(_)) => {
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

    if drained_clean
        && stream.stalled.is_none()
        && (stream.peer_finished || conn.stream_finished(stream_id))
    {
        stream.stream_tx = None;
        return true;
    }
    false
}

/// Retry a stalled chunk and emit EOF for every streaming response once its peer has finished.
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
            continue;
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

/// Deliver a terminal error to a streaming consumer reliably.
pub(super) fn deliver_stream_error(tx: &mpsc::Sender<std::io::Result<Bytes>>, err: std::io::Error) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let _ = tx.send(Err(err)).await;
    });
}

/// Fail every in-flight and queued request with `reason` and mark the connection closed.
pub(super) fn fail_all(
    streams: &mut HashMap<u64, H3Stream>,
    pending: &mut VecDeque<H3Command>,
    closed: &AtomicBool,
    reason: String,
) {
    closed.store(true, Ordering::Release);
    for (_, mut stream) in streams.drain() {
        if stream.head_sent {
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
