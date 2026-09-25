use super::*;

mod drain;
mod forward;
mod upload;
pub(super) use forward::*;
pub(super) use upload::*;

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
            priority_update,
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
                priority_update,
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
                            drop(resp_tx.send(Err(H3SendError::NotSent(
                                "server sent GOAWAY: request not sent".into(),
                            ))));
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
                                    drop(resp_tx.send(Err(H3SendError::NotSent(
                                        "server sent GOAWAY: request not sent".into(),
                                    ))));
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
    priority_update: bool,
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
        let expects_body = !headers
            .iter()
            .any(|h| h.name() == b":method" && h.value() == b"HEAD");
        let priority = priority_update
            .then(|| headers.iter().find(|h| h.name() == b"priority"))
            .flatten()
            .map(|h| h.value().to_vec());
        match h3.send_request(conn, headers, fin) {
            Ok(stream_id) => {
                if let Some(value) = priority
                    && let Err(e) = h3.send_priority_update_field_value(conn, stream_id, &value)
                {
                    tracing::debug!(
                        target: "leyline::quic",
                        stream_id,
                        error = %e,
                        "h3 PRIORITY_UPDATE not sent"
                    );
                }
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
                stream.expects_body = expects_body;
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
                    drop(resp_tx.send(Err(H3SendError::Failed(format!("h3 send_request: {e}")))));
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
        drop(resp_tx.send(Err(H3SendError::NotSent(reason.clone()))));
    }
}
