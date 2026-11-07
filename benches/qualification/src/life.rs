use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use leyline::{Body, Kind, Resolver, Session, Timeout, Tls};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tokio::runtime::Builder;
use tokio::sync::watch;

use crate::hostile::acceptor;
use crate::probe::rss_kib;
use crate::{Options, Verdict, number};

mod rotation;

const ACCEPT_BACKLOG: u32 = 65535;
const SPREAD_SHARE: u64 = 16;
const CHUNK: usize = 64 * 1024;
const MIB: u64 = 1 << 20;
const FNV_START: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const SIZES: [u64; 3] = [MIB, 64 * MIB, 1024 * MIB];
const STREAM_RSS_LIMIT_KIB: u64 = 64 * 1024;

fn hold(args: &[String]) -> Duration {
    Duration::from_millis(number(args, "--hold-ms", 3_000))
}

fn digest(state: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(state, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

fn pattern() -> Arc<[u8]> {
    (0..CHUNK)
        .map(|i| (i % 251) as u8)
        .collect::<Vec<u8>>()
        .into()
}

fn pattern_digest(size: u64) -> u64 {
    let block = pattern();
    let mut state = FNV_START;
    let mut left = size;
    while left > 0 {
        let take = left.min(CHUNK as u64) as usize;
        state = digest(state, &block[..take]);
        left -= take as u64;
    }
    state
}

struct Head {
    post: bool,
    size: u64,
    length: u64,
    rest: Vec<u8>,
}

async fn read_head(stream: &mut TcpStream) -> Option<Head> {
    let mut buffer = Vec::with_capacity(1024);
    let end = loop {
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        let mut more = [0u8; 1024];
        let read = stream.read(&mut more).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&more[..read]);
    };
    let text = String::from_utf8_lossy(&buffer[..end]).into_owned();
    let line = text.lines().next()?;
    let path = line.split_whitespace().nth(1)?;
    let length = text.lines().find_map(|l| {
        let (key, value) = l.split_once(':')?;
        key.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse().ok())?
    });
    Some(Head {
        post: line.starts_with("POST"),
        size: path.rsplit('/').next()?.parse().unwrap_or(0),
        length: length.unwrap_or(0),
        rest: buffer[end..].to_vec(),
    })
}

async fn write_sized(stream: &mut TcpStream, size: u64) -> io::Result<()> {
    let block = pattern();
    stream
        .write_all(format!("HTTP/1.1 200 OK\r\ncontent-length: {size}\r\n\r\n").as_bytes())
        .await?;
    let mut left = size;
    while left > 0 {
        let take = left.min(CHUNK as u64) as usize;
        stream.write_all(&block[..take]).await?;
        left -= take as u64;
    }
    Ok(())
}

async fn read_upload(stream: &mut TcpStream, head: &Head) -> io::Result<u64> {
    let mut state = digest(FNV_START, &head.rest);
    let mut left = head.length - head.rest.len() as u64;
    let mut block = vec![0u8; CHUNK];
    while left > 0 {
        let want = left.min(CHUNK as u64) as usize;
        let read = stream.read(&mut block[..want]).await?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        state = digest(state, &block[..read]);
        left -= read as u64;
    }
    Ok(state)
}

async fn serve_one(mut stream: TcpStream, served: Arc<AtomicU64>) {
    while let Some(head) = read_head(&mut stream).await {
        served.fetch_add(1, Ordering::Relaxed);
        let done = if head.post {
            match read_upload(&mut stream, &head).await {
                Ok(state) => {
                    let body = state.to_string();
                    stream
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{body}",
                                body.len()
                            )
                            .as_bytes(),
                        )
                        .await
                }
                Err(error) => Err(error),
            }
        } else {
            write_sized(&mut stream, head.size).await
        };
        if done.is_err() {
            return;
        }
    }
}

async fn start_origin(host: &str, served: Arc<AtomicU64>) -> SocketAddr {
    let wanted: SocketAddr = format!("{host}:0").parse().expect("origin address");
    let socket = if wanted.is_ipv4() {
        TcpSocket::new_v4()
    } else {
        TcpSocket::new_v6()
    }
    .expect("socket");
    socket.bind(wanted).expect("bind");
    let listener = socket.listen(ACCEPT_BACKLOG).expect("listen");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            drop(stream.set_nodelay(true));
            tokio::spawn(serve_one(stream, Arc::clone(&served)));
        }
    });
    address
}

async fn download_digest(session: &Session, url: &str, pause: Duration) -> leyline::Result<u64> {
    let mut body = session.get(url).stream().send().await?.bytes_stream()?;
    let mut state = FNV_START;
    while let Some(chunk) = body.next().await {
        state = digest(state, &chunk.map_err(|error| error)?);
        if !pause.is_zero() {
            tokio::time::sleep(pause).await;
        }
    }
    Ok(state)
}

async fn upload_digest(session: &Session, url: &str, size: u64) -> leyline::Result<String> {
    let block = pattern();
    let chunks = (size / CHUNK as u64) as usize;
    let tail = (size % CHUNK as u64) as usize;
    let stream =
        futures_util::stream::iter((0..chunks + usize::from(tail > 0)).map(move |index| {
            let take = if index == chunks { tail } else { CHUNK };
            Ok::<_, io::Error>(bytes::Bytes::copy_from_slice(&block[..take]))
        }));
    session
        .post(url)
        .body(Body::stream(stream, Some(size)))
        .send()
        .await?
        .text()
        .await
}

async fn bodies(args: &[String], verdict: &Verdict) -> Value {
    let largest: u64 = number(args, "--max-bytes", u64::MAX);
    let address = start_origin("127.0.0.1", Arc::default()).await;
    let session = Session::builder()
        .max_body_size(usize::MAX)
        .timeout(Timeout::new().total(Duration::from_secs(120)))
        .build()
        .expect("session");
    let mut rows = Vec::new();
    for size in SIZES.into_iter().filter(|size| *size <= largest) {
        let url = format!("http://{address}/size/{size}");
        let expected = pattern_digest(size);
        let start_rss = rss_kib();
        let peak = Arc::new(AtomicU64::new(start_rss));
        let watching = watch_rss(Arc::clone(&peak));
        let began = Instant::now();
        let down = download_digest(&session, &url, Duration::ZERO).await;
        let down_seconds = began.elapsed().as_secs_f64();
        let down_growth = peak
            .swap(rss_kib(), Ordering::Relaxed)
            .saturating_sub(start_rss);
        let up = upload_digest(&session, &url, size).await;
        watching.abort();
        match &down {
            Ok(state) if *state == expected => {}
            other => verdict.fail(format!(
                "download {size}: digest {other:?}, expected {expected}"
            )),
        }
        match &up {
            Ok(text) if *text == expected.to_string() => {}
            other => verdict.fail(format!(
                "upload {size}: digest {other:?}, expected {expected}"
            )),
        }
        let up_growth = peak.load(Ordering::Relaxed).saturating_sub(start_rss);
        for (direction, growth) in [("download", down_growth), ("upload", up_growth)] {
            if growth > STREAM_RSS_LIMIT_KIB {
                verdict.fail(format!(
                    "memory: {growth} KiB over a {size} byte streamed {direction}"
                ));
            }
        }
        rows.push(json!({"bytes": size, "download_rss_growth_kib": down_growth, "upload_rss_growth_kib": up_growth, "download_seconds": down_seconds}));
    }
    json!({"transfers": rows})
}

fn watch_rss(peak: Arc<AtomicU64>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            peak.fetch_max(rss_kib(), Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
}

async fn slow_readers(args: &[String], verdict: &Verdict) -> Value {
    let readers: usize = number(args, "--readers", 10_000);
    let size: u64 = number(args, "--bytes", MIB);
    let address = start_origin("127.0.0.1", Arc::default()).await;
    let session = Session::builder()
        .max_connections(readers * 2)
        .max_connections_per_host(readers * 2)
        .timeout(Timeout::new().total(Duration::from_secs(120)))
        .build()
        .expect("session");
    let url = Arc::new(format!("http://{address}/size/{size}"));
    let expected = pattern_digest(size);
    let start_rss = rss_kib();
    let peak = Arc::new(AtomicU64::new(start_rss));
    let watching = watch_rss(Arc::clone(&peak));
    let wrong = Arc::new(AtomicU64::new(0));
    let mut tasks = Vec::with_capacity(readers);
    let began = Instant::now();
    for _ in 0..readers {
        let (session, url, wrong) = (session.clone(), Arc::clone(&url), Arc::clone(&wrong));
        tasks.push(tokio::spawn(async move {
            if !matches!(download_digest(&session, &url, Duration::from_millis(20)).await, Ok(state) if state == expected) {
                wrong.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for task in tasks {
        task.await.expect("reader");
    }
    watching.abort();
    let failed = wrong.load(Ordering::Relaxed);
    if failed > 0 {
        verdict.fail(format!(
            "{failed} of {readers} slow readers failed or saw a wrong digest"
        ));
    }
    let growth = peak.load(Ordering::Relaxed).saturating_sub(start_rss);
    json!({"readers": readers, "bytes_each": size, "failed": failed, "rss_growth_kib": growth,
           "kib_per_reader": growth as f64 / readers as f64, "seconds": began.elapsed().as_secs_f64()})
}

async fn kind_of(session: &Session, url: &str) -> String {
    match session.get(url).send().await {
        Ok(response) => match response.bytes().await {
            Ok(_) => "ok".to_owned(),
            Err(error) => error.kind().as_str().to_owned(),
        },
        Err(error) => error.kind().as_str().to_owned(),
    }
}

async fn stalled_origin() -> SocketAddr {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held.push(stream);
        }
    });
    address
}

async fn failures(args: &[String], verdict: &Verdict) -> Value {
    let ca_path = crate::hostile::text(args, "--ca")
        .unwrap_or("life-ca.der")
        .to_owned();
    let tls = acceptor(&ca_path);
    drop(std::fs::remove_file(&ca_path));
    let tls_listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let tls_address = tls_listener.local_addr().expect("address");
    tokio::spawn(async move {
        while let Ok((socket, _)) = tls_listener.accept().await {
            let tls = tls.clone();
            tokio::spawn(async move { drop(tls.accept(socket).await) });
        }
    });
    let refused = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        listener.local_addr().expect("address")
    };
    let healthy = start_origin("127.0.0.1", Arc::default()).await;
    let stalled = stalled_origin().await;
    let session = Session::builder()
        .tls(Tls::new().env_roots(false).system_roots(false))
        .timeout(Timeout::new().total(Duration::from_millis(500)))
        .build()
        .expect("session");
    let cases: [(&str, String, &str); 5] = [
        (
            "dns failure",
            "http://no-such-host.invalid/".to_owned(),
            Kind::Dns.as_str(),
        ),
        (
            "connection refused",
            format!("http://{refused}/"),
            Kind::Connect.as_str(),
        ),
        (
            "untrusted certificate",
            format!("https://{tls_address}/"),
            Kind::Tls.as_str(),
        ),
        (
            "no response before the deadline",
            format!("http://{stalled}/"),
            Kind::Timeout.as_str(),
        ),
        ("healthy host", format!("http://{healthy}/size/16"), "ok"),
    ];
    let mut rows = BTreeMap::new();
    for (name, url, expected) in &cases {
        let got = kind_of(&session, url).await;
        if got != *expected {
            verdict.fail(format!("{name}: kind {got}, expected {expected}"));
        }
        rows.insert((*name).to_owned(), got);
    }
    json!({"kinds": rows})
}

struct Flip {
    next: AtomicU8,
    addresses: [SocketAddr; 2],
}

impl Resolver for Flip {
    fn resolve<'a>(
        &'a self,
        _host: &'a str,
        _port: u16,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SocketAddr>, io::Error>> + Send + 'a>> {
        let address = self.addresses[self.next.load(Ordering::Relaxed) as usize];
        Box::pin(async move { Ok(vec![address]) })
    }
}

async fn dns_move(args: &[String], verdict: &Verdict) -> Value {
    let lifetime: u64 = number(args, "--max-lifetime-ms", 0);
    let settle = Duration::from_millis(number(args, "--settle-ms", 15_000));
    let hold = hold(args);
    let (old_served, new_served) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let old = start_origin("127.0.0.1", Arc::clone(&old_served)).await;
    let new = start_origin("127.0.0.2", Arc::clone(&new_served)).await;
    let flip = Arc::new(Flip {
        next: AtomicU8::new(0),
        addresses: [old, new],
    });
    let mut builder = Session::builder().dns_resolver(flip.clone() as Arc<dyn Resolver>);
    if lifetime > 0 {
        builder = builder.pool_max_lifetime(Duration::from_millis(lifetime));
    }
    let session = builder.build().expect("session");
    let failed = Arc::new(AtomicU64::new(0));
    let traffic = {
        let (session, failed) = (session.clone(), Arc::clone(&failed));
        tokio::spawn(async move {
            let mut every = tokio::time::interval(Duration::from_millis(1));
            loop {
                every.tick().await;
                let (session, failed) = (session.clone(), Arc::clone(&failed));
                tokio::spawn(async move {
                    if kind_of(&session, "http://origin.test/size/16").await != "ok" {
                        failed.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
        })
    };
    tokio::time::sleep(hold).await;
    flip.next.store(1, Ordering::Relaxed);
    tokio::time::sleep(settle).await;
    let (old_mark, new_mark) = (
        old_served.load(Ordering::Relaxed),
        new_served.load(Ordering::Relaxed),
    );
    tokio::time::sleep(hold).await;
    traffic.abort();
    let (old_tail, new_tail) = (
        old_served.load(Ordering::Relaxed) - old_mark,
        new_served.load(Ordering::Relaxed) - new_mark,
    );
    if old_tail > 0 {
        verdict.fail(format!("stale: {old_tail} requests still reached the old address {settle:?} after the DNS change, {new_tail} reached the new one"));
    }
    let failed = failed.load(Ordering::Relaxed);
    if failed > 0 {
        verdict.fail(format!("{failed} requests failed across the DNS change"));
    }
    json!({"old_address_after_settle": old_tail, "new_address_after_settle": new_tail, "failed": failed})
}

struct Fixed(Vec<SocketAddr>);

impl Resolver for Fixed {
    fn resolve<'a>(
        &'a self,
        _host: &'a str,
        _port: u16,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SocketAddr>, io::Error>> + Send + 'a>> {
        let addresses = self.0.clone();
        Box::pin(async move { Ok(addresses) })
    }
}

async fn start_killable(host: &str, served: Arc<AtomicU64>) -> (SocketAddr, watch::Sender<bool>) {
    let listener = TcpListener::bind((host, 0)).await.expect("bind");
    let address = listener.local_addr().expect("address");
    let (kill, killed) = watch::channel(false);
    let mut accepting = killed.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = accepting.changed() => return,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { return };
                    drop(stream.set_nodelay(true));
                    let (served, mut ending) = (Arc::clone(&served), killed.clone());
                    tokio::spawn(async move {
                        tokio::select! {
                            _ = ending.changed() => {}
                            () = serve_one(stream, served) => {}
                        }
                    });
                }
            }
        }
    });
    (address, kill)
}

#[derive(Default)]
struct Steady {
    good: AtomicU64,
    failed: AtomicU64,
    first_failure: Mutex<Option<String>>,
}

impl Steady {
    fn snapshot(&self) -> (u64, u64) {
        (
            self.good.load(Ordering::Relaxed),
            self.failed.load(Ordering::Relaxed),
        )
    }
}

fn steady_traffic(
    session: Session,
    url: String,
    every: Duration,
    tally: Arc<Steady>,
) -> tokio::task::JoinHandle<()> {
    let url = Arc::new(url);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(every);
        loop {
            tick.tick().await;
            let (session, url, tally) = (session.clone(), Arc::clone(&url), Arc::clone(&tally));
            tokio::spawn(async move {
                let reply = async { session.get(url.as_str()).send().await?.bytes().await }.await;
                match reply {
                    Ok(_) => {
                        tally.good.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) => {
                        tally.failed.fetch_add(1, Ordering::Relaxed);
                        tally
                            .first_failure
                            .lock()
                            .unwrap()
                            .get_or_insert_with(|| format!("{error:?}"));
                    }
                }
            });
        }
    })
}

async fn capacity_loss(args: &[String], verdict: &Verdict) -> Value {
    let settle = Duration::from_millis(number(args, "--settle-ms", 2_000));
    let hold = hold(args);
    let served: Vec<Arc<AtomicU64>> = (0..4).map(|_| Arc::default()).collect();
    let mut origins = Vec::new();
    for (index, count) in served.iter().enumerate() {
        origins.push(start_killable(&format!("127.0.0.{}", index + 1), Arc::clone(count)).await);
    }
    let addresses = origins.iter().map(|origin| origin.0).collect();
    let session = Session::builder()
        .dns_resolver(Arc::new(Fixed(addresses)) as Arc<dyn Resolver>)
        .retry(number::<u32>(args, "--retries", 2))
        .timeout(Timeout::new().total(Duration::from_secs(5)))
        .build()
        .expect("session");
    let tally = Arc::new(Steady::default());
    let traffic = steady_traffic(
        session,
        "http://origin.test/size/16".to_owned(),
        Duration::from_micros(500),
        Arc::clone(&tally),
    );
    tokio::time::sleep(hold).await;
    let before = tally.snapshot();
    let counts = |served: &[Arc<AtomicU64>]| {
        served
            .iter()
            .map(|c| c.load(Ordering::Relaxed))
            .collect::<Vec<u64>>()
    };
    let served_before = counts(&served);
    for origin in &origins[..2] {
        let _ = origin.1.send(true);
    }
    tokio::time::sleep(settle).await;
    let settled = tally.snapshot();
    let served_settled = counts(&served);
    tokio::time::sleep(hold).await;
    traffic.abort();
    let after = tally.snapshot();
    let served_after = counts(&served);
    let (lost_at_change, lost_after) = (settled.1 - before.1, after.1 - settled.1);
    let survivors: u64 = (2..4).map(|i| served_after[i] - served_settled[i]).sum();
    if lost_after > 0 {
        verdict.fail(format!(
            "{lost_after} requests failed {settle:?} or more after losing 2 of 4 addresses"
        ));
    }
    if lost_at_change > 0 {
        verdict.fail(format!(
            "{lost_at_change} requests failed when 2 of 4 addresses went away, first: {:?}",
            tally.first_failure.lock().unwrap()
        ));
    }
    if survivors == 0 {
        verdict.fail("no request reached the surviving addresses".to_owned());
    }
    let total_before: u64 = served_before.iter().sum();
    if served_before
        .iter()
        .any(|&served| served * SPREAD_SHARE < total_before)
    {
        verdict.fail(format!("spread: addresses served {served_before:?} before the loss, each needs 1/{SPREAD_SHARE} of the load"));
    }
    json!({"before": before, "good_after_settle": after.0 - settled.0, "lost_at_change": lost_at_change,
           "lost_after_settle": lost_after, "served_before": served_before, "served_after": served_after})
}

async fn shutdown_under_load(args: &[String], verdict: &Verdict) -> Value {
    let hold = hold(args);
    let address = start_origin("127.0.0.1", Arc::default()).await;
    let session = Session::builder()
        .timeout(Timeout::new().total(Duration::from_secs(10)))
        .build()
        .expect("session");
    let expected = pattern_digest(MIB);
    let url = Arc::new(format!("http://{address}/size/{MIB}"));
    let (finished, refused, wrong) = (
        Arc::new(AtomicU64::new(0)),
        Arc::new(AtomicU64::new(0)),
        Arc::new(AtomicU64::new(0)),
    );
    let mut tasks = Vec::new();
    for _ in 0..2000 {
        let (session, url, finished, refused, wrong) = (
            session.clone(),
            Arc::clone(&url),
            Arc::clone(&finished),
            Arc::clone(&refused),
            Arc::clone(&wrong),
        );
        tasks.push(tokio::spawn(async move {
            loop {
                match download_digest(&session, &url, Duration::ZERO).await {
                    Ok(state) if state == expected => {
                        finished.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(_) => {
                        wrong.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        refused.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                }
            }
        }));
    }
    tokio::time::sleep(hold).await;
    let before = finished.load(Ordering::Relaxed);
    let began = Instant::now();
    session.shutdown().await;
    for task in tasks {
        if tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .is_err()
        {
            verdict.fail("a request outlived shutdown by 5 s".to_owned());
            break;
        }
    }
    let waited = began.elapsed();
    let wrong = wrong.load(Ordering::Relaxed);
    if wrong > 0 {
        verdict.fail(format!("{wrong} finished requests had a wrong digest"));
    }
    if before == 0 {
        verdict.fail("no request finished before shutdown; the run says nothing".to_owned());
    }
    json!({"finished_before": before, "ended_by_shutdown": refused.load(Ordering::Relaxed),
           "seconds_to_quiet": waited.as_secs_f64(), "wrong": wrong})
}

pub fn run(options: &Options, args: &[String], verdict: &Verdict) -> Value {
    let case = crate::hostile::text(args, "--case")
        .unwrap_or("bodies")
        .to_owned();
    let threads = options.client_threads;
    let runtime = Builder::new_multi_thread()
        .worker_threads(threads)
        .enable_all()
        .build()
        .expect("runtime");
    let detail = runtime.block_on(async {
        match case.as_str() {
            "bodies" => bodies(args, verdict).await,
            "slow-readers" => slow_readers(args, verdict).await,
            "failures" => failures(args, verdict).await,
            "dns-move" => dns_move(args, verdict).await,
            "capacity-loss" => capacity_loss(args, verdict).await,
            "certificate-rotation" => rotation::certificate_rotation(args, verdict).await,
            "shutdown" => shutdown_under_load(args, verdict).await,
            other => panic!("unknown case {other}"),
        }
    });
    runtime.shutdown_background();
    json!({"case": case, "result": detail})
}
