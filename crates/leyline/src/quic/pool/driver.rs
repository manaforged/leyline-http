use super::*;

mod drain;

struct Drain<'a> {
    h3: &'a mut quiche::h3::Connection,
    conn: &'a mut quiche::Connection,
    streams: &'a mut HashMap<u64, H3Stream>,
    pending: &'a mut VecDeque<H3Command>,
    scratch: &'a mut [u8],
    max_body: u64,
    admit: &'a mut Option<usize>,
}

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
                match conn.close(true, 0x100, b"done") {
                    Ok(()) | Err(quiche::Error::Done) => {}
                    Err(e) => tracing::warn!(
                        target: "leyline::quic",
                        error = %e,
                        "h3 connection close failed"
                    ),
                }
                if let Err(e) = flush_egress(&socket, &mut conn, &mut out).await {
                    tracing::warn!(
                        target: "leyline::quic",
                        error = %e,
                        "h3 final egress flush failed"
                    );
                }
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
                            drop(resp_tx
                                .send(Err("server sent GOAWAY: request not sent".into())));
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
                                    drop(resp_tx.send(Err(
                                        "server sent GOAWAY: request not sent".into()
                                    )));
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
        retried,
        ..
    }) = pending.front()
    {
        if admit_cap.is_some_and(|cap| streams.len() >= cap) {
            break;
        }
        let streaming = body_stream.is_some();
        let fin = !streaming && body.as_ref().is_none_or(Bytes::is_empty);
        let retry = (!streaming && !*retried).then(|| (headers.clone(), body.clone()));
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
                    drop(resp_tx.send(Err(format!("h3 send_request: {e}"))));
                }
            }
        }
    }
}

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
                drop(
                    tx.send(H3BodyChunk::Eof {
                        stream_id,
                        error: Some(error),
                    })
                    .await,
                );
                return;
            }
        }
    }
    drop(
        tx.send(H3BodyChunk::Eof {
            stream_id,
            error: None,
        })
        .await,
    );
}

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
                    shutdown(conn, stream_id, quiche::Shutdown::Write, 0);
                    shutdown(conn, stream_id, quiche::Shutdown::Read, 0);
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

fn shutdown(conn: &mut quiche::Connection, id: u64, dir: quiche::Shutdown, err: u64) {
    match conn.stream_shutdown(id, dir, err) {
        Ok(()) | Err(quiche::Error::Done) => {}
        Err(e) => tracing::warn!(
            target: "leyline::quic",
            stream_id = id,
            error = %e,
            "h3 stream shutdown failed"
        ),
    }
}

fn reset_upload_half(conn: &mut quiche::Connection, stream_id: u64, stream: &mut H3Stream) {
    if stream.send_side_open() {
        shutdown(conn, stream_id, quiche::Shutdown::Write, 0);
    }
    stream.cancel_upload();
}

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

pub(super) fn cancelled_stream_ids(streams: &HashMap<u64, H3Stream>) -> Vec<u64> {
    streams
        .iter()
        .filter(|(_, s)| stream_is_cancelled(s))
        .map(|(&id, _)| id)
        .collect()
}

pub(super) fn sweep_cancelled_streams(
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
) {
    for id in cancelled_stream_ids(streams) {
        if let Some(mut stream) = streams.remove(&id) {
            shutdown(conn, id, quiche::Shutdown::Read, 0);
            reset_upload_half(conn, id, &mut stream);
        }
    }
}

pub(super) fn drain_h3_events(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
    pending: &mut VecDeque<H3Command>,
    scratch: &mut [u8],
    max_response_body_bytes: u64,
    admit_cap: &mut Option<usize>,
) -> Result<bool, String> {
    Drain {
        h3,
        conn,
        streams,
        pending,
        scratch,
        max_body: max_response_body_bytes,
        admit: admit_cap,
    }
    .run()
}

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
                shutdown(conn, stream_id, quiche::Shutdown::Read, 0);
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
                    shutdown(
                        conn,
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
                        shutdown(conn, stream_id, quiche::Shutdown::Read, 0);
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

pub(super) fn deliver_stream_error(tx: &mpsc::Sender<std::io::Result<Bytes>>, err: std::io::Error) {
    let tx = tx.clone();
    tokio::spawn(async move {
        drop(tx.send(Err(err)).await);
    });
}

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
        drop(resp_tx.send(Err(reason.clone())));
    }
}
