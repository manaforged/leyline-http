use super::*;

pub(super) async fn run_h1_stream_pump(mut pump: H1StreamPump) {
    let initial = std::mem::take(&mut pump.initial_body);
    let excess = has_excess(pump.framing, initial.len());
    let drain = Drain {
        limit: pump.drain_limit,
        wait: pump.pool.config.idle_timeout,
    };
    let drained_clean =
        stream_body_into(pump.io.as_mut(), pump.framing, initial, &pump.tx, drain).await;
    if drained_clean && pump.reusable && !excess {
        pump.pool
            .return_h1(&pump.key, H1Slot::new(pump.io), pump.tls);
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
        let line_end = line.map_err(Error::into_io)?;
        let mut remaining = parse::chunk_size(&buf[..line_end])
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        buf.drain(..line_end + 2);

        if remaining == 0 {
            return match until_closed(tx, read_chunk_trailers(stream, &mut buf)).await {
                Some(trailers) => trailers.map(|()| true).map_err(Error::into_io),
                None => Ok(false),
            };
        }

        while remaining > 0 {
            if buf.is_empty() {
                let Some(more) = until_closed(tx, read_more(stream, &mut buf)).await else {
                    return Ok(false);
                };
                more.map_err(Error::into_io)?;
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
        tail.map_err(Error::into_io)?;
        if &buf[..2] != b"\r\n" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk missing CRLF terminator",
            ));
        }
        buf.drain(..2);
    }
}
pub(super) struct Exchange<'a> {
    pub(super) pool: &'a Arc<Pool>,
    pub(super) key: &'a PoolKey,
    pub(super) req: &'a H1Request<'a>,
    pub(super) permit: Option<OwnedSemaphorePermit>,
}

impl Exchange<'_> {
    pub(super) async fn on(
        &mut self,
        slot: H1Slot,
        tls: &TlsInfo,
        body: Body,
        count_install: bool,
    ) -> Result<Delivered, Error> {
        let req = self.req;
        let H1Slot { mut io, buf } = slot;
        let (head, reusable) = exchange_head_on_stream(
            io.as_mut(),
            req.method,
            req.url,
            Fields {
                pairs: &req.headers,
                lines: req.headers.h1_lines(),
            },
            body,
            req.target,
            buf,
        )
        .await?;
        let leg = HeadLeg {
            io,
            tls: tls.clone(),
            head,
            reusable,
            count_install,
        };
        deliver_head(self.pool, self.key, leg, req.response, &mut self.permit).await
    }
}

pub(super) struct Delivered {
    status: u16,
    headers: crate::core::response_head::Fields,
    body: TransportBody,
}

impl Delivered {
    pub(super) fn respond(
        self,
        scheme: &str,
        tls: TlsInfo,
        timing: ResponseTiming,
    ) -> Result<H1Outcome, Error> {
        let status = crate::core::transport::status(self.status)?;
        Ok(H1Outcome::Response(TransportResponse {
            status,
            headers: self.headers,
            trailers: None,
            body: self.body,
            version: http::Version::HTTP_11,
            tls: (scheme == "https").then_some(tls),
            timing,
        }))
    }
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
    key: &PoolKey,
    leg: HeadLeg,
    response: ResponseMode,
    permit: &mut Option<OwnedSemaphorePermit>,
) -> Result<Delivered, Error> {
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
            let buf = std::mem::take(&mut head.initial_body);
            pool.return_h1(key, H1Slot { io, buf }, tls);
            if count_install {
                pool.note_h1_install();
            }
        }
        return Ok(Delivered {
            status,
            headers,
            body: TransportBody::Buffered(body),
        });
    }
    let (tx, rx) = mpsc::channel(STREAM_CHANNEL_DEPTH);
    let pump = run_h1_stream_pump(H1StreamPump {
        io,
        permit: permit.take(),
        pool: pool.clone(),
        key: key.clone(),
        tls,
        framing: head.framing,
        initial_body: head.initial_body,
        reusable,
        count_install,
        drain_limit: response
            .error_budget()
            .map_or(0, |budget| budget.bytes as u64),
        tx,
    });
    let body = BodyStream::new(rx);
    let body = if pool.config.blocking.is_some() {
        body.pumped_by(pump)
    } else {
        tokio::spawn(pump);
        body
    };
    Ok(Delivered {
        status,
        headers,
        body: TransportBody::Streaming(Box::new(body)),
    })
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
