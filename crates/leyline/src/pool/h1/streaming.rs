use super::*;

pub(super) async fn run_h1_stream_pump(mut pump: H1StreamPump) {
    let initial = std::mem::take(&mut pump.initial_body);
    let excess = has_excess(pump.framing, initial.len());
    let drained_clean = stream_body_into(pump.io.as_mut(), pump.framing, initial, &pump.tx).await;
    if drained_clean && pump.reusable && !excess {
        pump.pool
            .return_h1(pump.key, H1Slot { io: pump.io }, pump.tls);
        if pump.count_install {
            pump.pool.note_h1_install();
        }
    }
    drop(pump.permit);
}
pub(super) async fn stream_body_into(
    stream: &mut dyn H1Io,
    framing: BodyFraming,
    initial: Vec<u8>,
    tx: &mpsc::Sender<io::Result<Bytes>>,
) -> bool {
    let result = match framing {
        BodyFraming::None => Ok(true),
        BodyFraming::Fixed(len) => stream_fixed_into(stream, initial, len, tx).await,
        BodyFraming::Chunked => stream_chunked_into(stream, initial, tx).await,
        BodyFraming::ToClose => stream_to_close_into(stream, initial, tx).await,
    };
    match result {
        Ok(clean) => clean,
        Err(e) => {
            let _ = tx.send(Err(e)).await;
            false
        }
    }
}
async fn until_closed<T>(
    tx: &mpsc::Sender<io::Result<Bytes>>,
    work: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        () = tx.closed() => None,
        out = work => Some(out),
    }
}
pub(super) async fn stream_fixed_into(
    stream: &mut dyn H1Io,
    initial: Vec<u8>,
    len: u64,
    tx: &mpsc::Sender<io::Result<Bytes>>,
) -> io::Result<bool> {
    let mut remaining = len;
    if !initial.is_empty() {
        let take = (initial.len() as u64).min(remaining) as usize;
        if take > 0 {
            if tx
                .send(Ok(Bytes::copy_from_slice(&initial[..take])))
                .await
                .is_err()
            {
                return Ok(false);
            }
            remaining -= take as u64;
        }
    }
    let mut tmp = vec![0u8; 8192];
    while remaining > 0 {
        let want = remaining.min(tmp.len() as u64) as usize;
        let Some(read) = until_closed(tx, stream.read(&mut tmp[..want])).await else {
            return Ok(false);
        };
        let n = read?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before HTTP/1.1 body completed",
            ));
        }
        if tx
            .send(Ok(Bytes::copy_from_slice(&tmp[..n])))
            .await
            .is_err()
        {
            return Ok(false);
        }
        remaining -= n as u64;
    }
    Ok(true)
}
pub(super) async fn stream_to_close_into(
    stream: &mut dyn H1Io,
    initial: Vec<u8>,
    tx: &mpsc::Sender<io::Result<Bytes>>,
) -> io::Result<bool> {
    if !initial.is_empty() && tx.send(Ok(Bytes::from(initial))).await.is_err() {
        return Ok(false);
    }
    let mut tmp = vec![0u8; 8192];
    loop {
        let Some(read) = until_closed(tx, stream.read(&mut tmp)).await else {
            return Ok(false);
        };
        let n = read?;
        if n == 0 {
            return Ok(true);
        }
        if tx
            .send(Ok(Bytes::copy_from_slice(&tmp[..n])))
            .await
            .is_err()
        {
            return Ok(false);
        }
    }
}
pub(super) async fn stream_chunked_into(
    stream: &mut dyn H1Io,
    mut buf: Vec<u8>,
    tx: &mpsc::Sender<io::Result<Bytes>>,
) -> io::Result<bool> {
    loop {
        let Some(line) = until_closed(tx, read_until_crlf(stream, &mut buf)).await else {
            return Ok(false);
        };
        let line_end = line.map_err(h1err_to_io)?;
        let size_line = String::from_utf8_lossy(&buf[..line_end]);
        let size_token = size_line.split(';').next().unwrap_or("").trim();
        let mut remaining = u64::from_str_radix(size_token, 16).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid chunk size: {e}"),
            )
        })?;
        buf.drain(..line_end + 2);

        if remaining == 0 {
            return match until_closed(tx, read_chunk_trailers(stream, &mut buf)).await {
                Some(trailers) => trailers.map(|()| true).map_err(h1err_to_io),
                None => Ok(false),
            };
        }

        while remaining > 0 {
            if buf.is_empty() {
                let Some(more) = until_closed(tx, read_more(stream, &mut buf)).await else {
                    return Ok(false);
                };
                more.map_err(h1err_to_io)?;
            }
            let take = usize::try_from(remaining).map_or(buf.len(), |wanted| wanted.min(buf.len()));
            let piece = Bytes::copy_from_slice(&buf[..take]);
            buf.drain(..take);
            remaining -= take as u64;
            if tx.send(Ok(piece)).await.is_err() {
                return Ok(false);
            }
        }

        let Some(tail) = until_closed(tx, read_until_available(stream, &mut buf, 2)).await else {
            return Ok(false);
        };
        tail.map_err(h1err_to_io)?;
        if &buf[..2] != b"\r\n" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk missing CRLF terminator",
            ));
        }
        buf.drain(..2);
    }
}
#[expect(
    clippy::too_many_arguments,
    reason = "flat per-request wire fields across one internal call path"
)]
pub(super) async fn send_request_h1_streaming(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    req: H1Request<'_>,
    body: H1Body,
    permit: OwnedSemaphorePermit,
    key: PoolKey,
    dial: H1Dial<'_>,
    legs: FirstLegs,
) -> Result<H1Outcome, H1PooledError> {
    let H1Request {
        scheme,
        method,
        url,
        headers,
        target,
        ..
    } = req;
    let started = legs.started;
    let replay = replay_body(&body);
    let mut body = body;

    if let Some((slot, tls)) = legs.pooled {
        let pooled_body = std::mem::replace(&mut body, H1Body::Empty);
        let mut io = slot.io;
        match exchange_head_on_stream(
            io.as_mut(),
            method,
            url,
            headers.clone(),
            pooled_body,
            target,
        )
        .await
        {
            Ok((head, reusable)) => {
                tracing::Span::current().record("pool.hit", true);
                let (tx, rx) = mpsc::channel(STREAM_CHANNEL_DEPTH);
                tokio::spawn(run_h1_stream_pump(H1StreamPump {
                    io,
                    permit,
                    pool: pool.clone(),
                    key,
                    tls: tls.clone(),
                    framing: head.framing,
                    initial_body: head.initial_body,
                    reusable,
                    count_install: false,
                    tx,
                }));
                return Ok(H1Outcome::Response(H1Response {
                    status: head.status,
                    headers: head.headers,
                    body: H1ResponseBody::Streaming(BodyStream::new(rx)),
                    tls: tls_for_scheme(scheme, &tls),
                    timing: ResponseTiming::leg(started, None),
                }));
            }
            Err(e) => body = resend_after_failure(pool, &key, method, replay, e)?,
        }
    }
    tracing::Span::current().record("pool.hit", false);

    let (mut slot, tls, connect_ms) =
        match fresh_leg(pool, &key, connector, dial, legs.fresh).await? {
            Leg::H1(slot, tls, connect_ms) => (slot, tls, connect_ms),
            Leg::H2(opened) => return Ok(H1Outcome::Upgraded { opened, body }),
        };
    let (head, reusable) =
        exchange_head_on_stream(slot.io.as_mut(), method, url, headers, body, target).await?;
    let (tx, rx) = mpsc::channel(STREAM_CHANNEL_DEPTH);
    tokio::spawn(run_h1_stream_pump(H1StreamPump {
        io: slot.io,
        permit,
        pool: pool.clone(),
        key,
        tls: tls.clone(),
        framing: head.framing,
        initial_body: head.initial_body,
        reusable,
        count_install: true,
        tx,
    }));
    Ok(H1Outcome::Response(H1Response {
        status: head.status,
        headers: head.headers,
        body: H1ResponseBody::Streaming(BodyStream::new(rx)),
        tls: tls_for_scheme(scheme, &tls),
        timing: ResponseTiming::leg(started, Some(connect_ms)),
    }))
}
