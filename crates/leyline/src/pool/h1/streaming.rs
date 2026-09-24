use super::*;

pub(super) async fn run_h1_stream_pump(mut pump: H1StreamPump) {
    let initial = std::mem::take(&mut pump.initial_body);
    let limit = pump.pool.max_body_size;
    let drained_clean =
        stream_body_into(pump.io.as_mut(), pump.framing, initial, &pump.tx, limit).await;
    if drained_clean && pump.reusable {
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
    limit: usize,
) -> bool {
    let result = match framing {
        BodyFraming::None => Ok(true),
        BodyFraming::Fixed(len) => stream_fixed_into(stream, initial, len, tx, limit).await,
        BodyFraming::Chunked => stream_chunked_into(stream, initial, tx, limit).await,
        BodyFraming::ToClose => stream_to_close_into(stream, initial, tx, limit).await,
    };
    match result {
        Ok(clean) => clean,
        Err(e) => {
            let _ = tx.send(Err(e)).await;
            false
        }
    }
}
pub(super) async fn stream_fixed_into(
    stream: &mut dyn H1Io,
    initial: Vec<u8>,
    len: u64,
    tx: &mpsc::Sender<io::Result<Bytes>>,
    limit: usize,
) -> io::Result<bool> {
    if len > limit as u64 {
        return Err(body_too_large_io(limit));
    }
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
        let n = stream.read(&mut tmp[..want]).await?;
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
    limit: usize,
) -> io::Result<bool> {
    let mut total = initial.len();
    if total > limit {
        return Err(body_too_large_io(limit));
    }
    if !initial.is_empty() && tx.send(Ok(Bytes::from(initial))).await.is_err() {
        return Ok(false);
    }
    let mut tmp = vec![0u8; 8192];
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(true);
        }
        total += n;
        if total > limit {
            return Err(body_too_large_io(limit));
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
    limit: usize,
) -> io::Result<bool> {
    let mut total: usize = 0;
    loop {
        let line_end = read_until_crlf(stream, &mut buf)
            .await
            .map_err(h1err_to_io)?;
        let size_line = String::from_utf8_lossy(&buf[..line_end]);
        let size_token = size_line.split(';').next().unwrap_or("").trim();
        let size_u64 = u64::from_str_radix(size_token, 16).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid chunk size: {e}"),
            )
        })?;
        if size_u64 > limit as u64 {
            return Err(body_too_large_io(limit));
        }
        let size = size_u64 as usize;
        buf.drain(..line_end + 2);

        if size == 0 {
            read_chunk_trailers(stream, &mut buf)
                .await
                .map_err(h1err_to_io)?;
            return Ok(true);
        }

        read_until_available(stream, &mut buf, size + 2)
            .await
            .map_err(h1err_to_io)?;
        total += size;
        if total > limit {
            return Err(body_too_large_io(limit));
        }
        if &buf[size..size + 2] != b"\r\n" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk missing CRLF terminator",
            ));
        }
        let chunk = Bytes::copy_from_slice(&buf[..size]);
        buf.drain(..size + 2);
        if tx.send(Ok(chunk)).await.is_err() {
            return Ok(false);
        }
    }
}
#[expect(
    clippy::too_many_arguments,
    reason = "flat per-request wire fields across one internal call path"
)]
pub(super) async fn send_request_h1_streaming(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    scheme: &str,
    host: &str,
    port: u16,
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: H1Body,
    proxy: Option<&str>,
    target: H1Target,
    permit: OwnedSemaphorePermit,
    key: PoolKey,
) -> Result<H1Response, H1PooledError> {
    let replay = replay_body(method, &body);
    let mut body = body;

    if let Some((slot, tls)) = checkout_live_h1(pool, &key) {
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
                return Ok(H1Response {
                    status: head.status,
                    headers: head.headers,
                    body: H1ResponseBody::Streaming(BodyStream::new(rx)),
                    tls: tls_for_scheme(scheme, &tls),
                });
            }
            Err(e) => {
                tracing::info!(
                    target: "leyline::pool",
                    host = %key.host,
                    port = key.port,
                    error = %e,
                    "pool stale hit -- pooled h1 stream failed before response, opening fresh"
                );
                pool.note_h1_dead();
                match replay {
                    Some(replay) => body = replay,
                    None => return Err(e),
                }
            }
        }
    }
    tracing::Span::current().record("pool.hit", false);

    let (io, tls): (Box<dyn H1Io>, TlsInfo) =
        open_new(connector, scheme, host, port, proxy).await?;
    let mut slot = H1Slot { io };
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
    Ok(H1Response {
        status: head.status,
        headers: head.headers,
        body: H1ResponseBody::Streaming(BodyStream::new(rx)),
        tls: tls_for_scheme(scheme, &tls),
    })
}

fn body_too_large_io(limit: usize) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("HTTP/1.1 body exceeds {limit} bytes"),
    )
}
