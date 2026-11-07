use super::*;

pub(super) async fn run_h1_stream_pump(mut pump: H1StreamPump) {
    let initial = std::mem::take(&mut pump.initial_body);
    let excess = has_excess(pump.framing, initial.len());
    let drain = Drain {
        limit: pump.drain_limit,
        wait: pump.pool.drain_wait(),
    };
    let drained_clean =
        stream_body_into(pump.io.as_mut(), pump.framing, initial, &pump.tx, drain).await;
    if drained_clean
        && pump.reusable
        && !excess
        && pump
            .pool
            .return_h1(&pump.key, H1Slot::new(pump.io), pump.tls)
        && pump.count_install
    {
        pump.pool.note(Counter::Installs);
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
    let mut buf = initial;
    while remaining > 0 {
        buf.clear();
        let want = usize::try_from(remaining).map_or(READ_CHUNK, |left| left.min(READ_CHUNK));
        let Some(read) = until_closed(tx, read_into(stream, &mut buf, want)).await else {
            return Ok(drain_fixed(stream, remaining, drain).await);
        };
        let n = read?;
        if n == 0 {
            return Err(cut_short().into_error().into_io());
        }
        remaining -= n as u64;
        if tx
            .send(Ok(Bytes::copy_from_slice(&buf[..n])))
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
    let mut tmp = vec![0u8; READ_CHUNK];
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
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let Some(read) = until_closed(tx, read_into(stream, &mut buf, READ_CHUNK)).await else {
            return Ok(false);
        };
        let n = read?;
        if n == 0 {
            return Ok(true);
        }
        if tx
            .send(Ok(Bytes::copy_from_slice(&buf[..n])))
            .await
            .is_err()
        {
            return Ok(false);
        }
    }
}
fn chunk_io(error: H1Error) -> io::Error {
    let error = error.into_error();
    match error.message() {
        Some(message) if is_malformed(&error) => {
            io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
        }
        _ => error.into_io(),
    }
}

pub(super) async fn stream_chunked_into(
    stream: &mut dyn H1Io,
    mut buf: Vec<u8>,
    tx: &mpsc::Sender<io::Result<Bytes>>,
) -> io::Result<bool> {
    let mut chunked = Chunked::new();
    loop {
        let Some(step) = until_closed(tx, chunked.next(stream, &mut buf)).await else {
            return Ok(false);
        };
        match step.map_err(chunk_io)? {
            Step::Size(_) => {}
            Step::Data(n) => {
                let piece = Bytes::copy_from_slice(&buf[..n]);
                buf.drain(..n);
                if tx.send(Ok(piece)).await.is_err() {
                    return Ok(false);
                }
            }
            Step::Done => return Ok(true),
        }
    }
}
pub(super) struct Exchange<'a> {
    pub(super) pool: &'a Arc<Pool>,
    pub(super) key: &'a PoolKey,
    pub(super) req: &'a H1Request<'a>,
    pub(super) started: Instant,
    pub(super) parked: bool,
}

impl Exchange<'_> {
    pub(super) async fn send<W>(
        &self,
        opened: Opened<H1Slot>,
        body: Body,
        first: bool,
    ) -> Result<Sent<Reply<W>>, Error> {
        let leg = opened.leg();
        let count_install = leg == LegKind::Fresh && !(first && self.parked);
        let replay = body.replay();
        let attempt = Attempt {
            leg,
            first,
            method: self.req.method,
        };
        match self.on(opened, body, count_install).await {
            Ok(delivered) => Ok(Sent::Done(Reply::Answered(delivered))),
            Err(e) => {
                resend_after_failure(self.pool, self.key, attempt, replay, e).map(Sent::Retry)
            }
        }
    }

    async fn on(
        &self,
        opened: Opened<H1Slot>,
        body: Body,
        count_install: bool,
    ) -> Result<Box<Delivered>, H1Error> {
        let req = self.req;
        let connect_ms = opened.connect_ms;
        let Opened {
            conn: H1Slot { mut io, buf },
            tls,
            ..
        } = opened;
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
        let answer = deliver_head(self.pool, self.key, leg, req.response, req.body_limit).await?;
        Ok(Box::new(Delivered {
            answer,
            tls,
            timing: ResponseTiming::leg(self.started, connect_ms),
        }))
    }
}

pub(super) struct Delivered {
    answer: Answer,
    tls: TlsInfo,
    timing: ResponseTiming,
}

struct Answer {
    status: u16,
    headers: crate::wire::Fields,
    trailers: http::HeaderMap,
    body: Delivery,
}

enum Delivery {
    Buffered(Vec<u8>),
    Streaming(mpsc::Receiver<io::Result<Bytes>>, Box<H1StreamPump>),
}

impl Delivered {
    pub(super) fn respond(
        self: Box<Self>,
        scheme: Scheme,
        permit: OwnedSemaphorePermit,
    ) -> Result<TransportResponse, Error> {
        let Answer {
            status,
            headers,
            trailers,
            body,
        } = self.answer;
        let status = crate::wire::status(status)?;
        let body = match body {
            Delivery::Buffered(body) => TransportBody::Buffered(body),
            Delivery::Streaming(rx, mut pump) => {
                pump.permit = Some(permit);
                let blocking = matches!(pump.pool.io(), crate::pool::Io::Blocking(_));
                let pump = run_h1_stream_pump(*pump);
                let body = crate::wire::BodyChannel::new(rx);
                TransportBody::Streaming(if blocking {
                    body.pumped_by(pump)
                } else {
                    tokio::spawn(pump);
                    body
                })
            }
        };
        let mut response =
            TransportResponse::new(http::Version::HTTP_11, status, headers, trailers, body);
        response.tls = scheme.is_secure().then_some(self.tls);
        response.timing = self.timing;
        Ok(response)
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
    body_limit: BodyLimit,
) -> Result<Answer, H1Error> {
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
                read_h1_body(io.as_mut(), &mut head, body_limit).await?,
                true,
            ),
        };
        let Buffered {
            body,
            trailers,
            excess,
        } = body;
        if complete && reusable && !excess {
            let buf = std::mem::take(&mut head.initial_body);
            if pool.return_h1(key, H1Slot { io, buf }, tls) && count_install {
                pool.note(Counter::Installs);
            }
        }
        return Ok(Answer {
            status,
            headers,
            trailers,
            body: Delivery::Buffered(body),
        });
    }
    let (tx, rx) = mpsc::channel(crate::wire::STREAM_CHANNEL_DEPTH);
    let pump = H1StreamPump {
        io,
        permit: None,
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
    };
    Ok(Answer {
        status,
        headers,
        trailers: http::HeaderMap::new(),
        body: Delivery::Streaming(rx, Box::new(pump)),
    })
}

async fn read_error_prefix(
    io: &mut dyn H1Io,
    head: &mut H1Head,
    budget: ErrorBudget,
    len: usize,
) -> (Buffered, bool) {
    let mut body = std::mem::take(&mut head.initial_body);
    let excess = has_excess(head.framing, body.len());
    let complete = matches!(
        within(Some(budget.wait), read_fixed_into(io, &mut body, len)).await,
        Ok(Ok(()))
    );
    body.truncate(len);
    let body = Buffered {
        body,
        trailers: http::HeaderMap::new(),
        excess,
    };
    (body, complete)
}
