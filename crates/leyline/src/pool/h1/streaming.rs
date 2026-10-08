use super::*;

pub(super) async fn run_h1_stream_pump(mut pump: H1StreamPump) {
    let initial = std::mem::take(&mut pump.initial_body);
    let excess = has_excess(pump.framing, initial.len());
    let drain = Drain {
        limit: pump.drain_limit,
        wait: pump.pool.idle_timeout,
    };
    let drained_clean =
        stream_body_into(pump.io.as_mut(), pump.framing, initial, &pump.tx, drain).await;
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
    drain: Drain,
) -> bool {
    let result = match framing {
        BodyFraming::None => Ok(true),
        BodyFraming::Fixed(len) => stream_fixed_into(stream, initial, len, tx, drain).await,
        BodyFraming::Chunked => stream_chunked_into(stream, initial, tx).await,
        BodyFraming::ToClose => stream_to_close_into(stream, initial, tx).await,
    };
    match result {
        Ok(clean) => clean,
        Err(e) => {
            drop(tx.send(Err(e)).await);
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
#[derive(Clone, Copy)]
pub(super) struct Drain {
    limit: u64,
    wait: Duration,
}

pub(super) async fn stream_fixed_into(
    stream: &mut dyn H1Io,
    initial: Vec<u8>,
    len: u64,
    tx: &mpsc::Sender<io::Result<Bytes>>,
    drain: Drain,
) -> io::Result<bool> {
    let take = (initial.len() as u64).min(len) as usize;
    let mut remaining = len - take as u64;
    if take > 0
        && tx
            .send(Ok(Bytes::copy_from_slice(&initial[..take])))
            .await
            .is_err()
    {
        return Ok(drain_fixed(stream, remaining, drain).await);
    }
    let mut tmp = vec![0u8; 8192];
    while remaining > 0 {
        let want = remaining.min(tmp.len() as u64) as usize;
        let Some(read) = until_closed(tx, stream.read(&mut tmp[..want])).await else {
            return Ok(drain_fixed(stream, remaining, drain).await);
        };
        let n = read?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before HTTP/1.1 body completed",
            ));
        }
        remaining -= n as u64;
        if tx
            .send(Ok(Bytes::copy_from_slice(&tmp[..n])))
            .await
            .is_err()
        {
            return Ok(drain_fixed(stream, remaining, drain).await);
        }
    }
    Ok(true)
}

async fn drain_fixed(stream: &mut dyn H1Io, mut remaining: u64, drain: Drain) -> bool {
    if remaining > drain.limit {
        return false;
    }
    let mut tmp = vec![0u8; 8192];
    let read_rest = async {
        while remaining > 0 {
            let want = remaining.min(tmp.len() as u64) as usize;
            match stream.read(&mut tmp[..want]).await {
                Ok(0) | Err(_) => return false,
                Ok(n) => remaining -= n as u64,
            }
        }
        true
    };
    within(Some(drain.wait), read_rest).await.unwrap_or(false)
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
        host,
        port,
        method,
        url,
        headers,
        target,
        response,
        ..
    } = req;
    let started = legs.started;
    let replay = replay_body(&body);
    let mut body = body;
    let mut permit = Some(permit);

    if let Some((slot, tls)) = legs.pooled {
        trace::connect(host, port, true, Duration::ZERO);
        let pooled_body = std::mem::replace(&mut body, H1Body::Empty);
        let mut io = slot.io;
        let delivered = match exchange_head_on_stream(
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
                let leg = HeadLeg {
                    io,
                    tls: tls.clone(),
                    head,
                    reusable,
                    count_install: false,
                };
                deliver_head(pool, key.clone(), leg, response, &mut permit).await
            }
            Err(e) => Err(e),
        };
        match delivered {
            Ok((status, headers, body)) => {
                tracing::Span::current().record("pool.hit", true);
                return Ok(H1Outcome::Response(H1Response {
                    status,
                    headers,
                    body,
                    tls: tls_for_scheme(scheme, &tls),
                    timing: ResponseTiming::leg(started, None),
                }));
            }
            Err(e) => body = resend_after_failure(pool, &key, method, replay, e)?,
        }
    }
    tracing::Span::current().record("pool.hit", false);
    let installed = legs.fresh.is_some();

    let (mut slot, tls, connect_ms) =
        match fresh_leg(pool, &key, connector, dial, legs.fresh).await? {
            Leg::H1(slot, tls, connect_ms) => (slot, tls, connect_ms),
            Leg::H2(opened) => return Ok(H1Outcome::Upgraded { opened, body }),
        };
    let (head, reusable) =
        exchange_head_on_stream(slot.io.as_mut(), method, url, headers, body, target).await?;
    let leg = HeadLeg {
        io: slot.io,
        tls: tls.clone(),
        head,
        reusable,
        count_install: !installed,
    };
    let (status, headers, body) = deliver_head(pool, key, leg, response, &mut permit).await?;
    Ok(H1Outcome::Response(H1Response {
        status,
        headers,
        body,
        tls: tls_for_scheme(scheme, &tls),
        timing: ResponseTiming::leg(started, Some(connect_ms)),
    }))
}

pub(super) struct HeadLeg {
    io: Box<dyn H1Io>,
    tls: TlsInfo,
    head: H1Head,
    reusable: bool,
    count_install: bool,
}

fn inline_error_len(response: ResponseMode, head: &H1Head) -> Option<(ErrorBudget, usize)> {
    let budget = response.error_budget()?;
    let len = match head.framing {
        BodyFraming::None => 0,
        BodyFraming::Fixed(len) => usize::try_from(len).ok()?,
        BodyFraming::Chunked | BodyFraming::ToClose => return None,
    };
    (len <= budget.bytes).then_some((budget, len))
}

async fn deliver_head(
    pool: &Arc<Pool>,
    key: PoolKey,
    leg: HeadLeg,
    response: ResponseMode,
    permit: &mut Option<OwnedSemaphorePermit>,
) -> Result<(u16, Vec<(String, String)>, H1ResponseBody), H1PooledError> {
    let HeadLeg {
        mut io,
        tls,
        mut head,
        reusable,
        count_install,
    } = leg;
    let status = head.status;
    let headers = std::mem::take(&mut head.headers);
    let inline_error = response
        .keeps_stream(status)
        .then(|| inline_error_len(response, &head))
        .flatten();
    if !response.keeps_stream(status) || inline_error.is_some() {
        let (body, complete) = match inline_error {
            Some((budget, len)) => read_error_prefix(io.as_mut(), &mut head, budget, len).await,
            None => (
                read_h1_body(io.as_mut(), &mut head, pool.max_body_size).await?,
                true,
            ),
        };
        let (body, excess) = body;
        if complete && reusable && !excess {
            pool.return_h1(key, H1Slot { io }, tls);
            if count_install {
                pool.note_h1_install();
            }
        }
        return Ok((status, headers, H1ResponseBody::Buffered(body)));
    }
    let (tx, rx) = mpsc::channel(STREAM_CHANNEL_DEPTH);
    tokio::spawn(run_h1_stream_pump(H1StreamPump {
        io,
        permit: permit.take(),
        pool: pool.clone(),
        key,
        tls,
        framing: head.framing,
        initial_body: head.initial_body,
        reusable,
        count_install,
        drain_limit: response
            .error_budget()
            .map_or(0, |budget| budget.bytes as u64),
        tx,
    }));
    Ok((
        status,
        headers,
        H1ResponseBody::Streaming(BodyStream::new(rx)),
    ))
}

async fn read_error_prefix(
    io: &mut dyn H1Io,
    head: &mut H1Head,
    budget: ErrorBudget,
    len: usize,
) -> ((Vec<u8>, bool), bool) {
    let mut body = std::mem::take(&mut head.initial_body);
    let excess = has_excess(head.framing, body.len());
    let complete = matches!(
        within(Some(budget.wait), read_fixed_into(io, &mut body, len)).await,
        Ok(Ok(()))
    );
    body.truncate(len);
    ((body, excess), complete)
}
