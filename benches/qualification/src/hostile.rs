use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hdrhistogram::Histogram;
use leyline::{Body, Protocol, Retry, Session, Timeout, Tls};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Builder;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;

use crate::{Options, Verdict, number};

const PREFACE_LEN: usize = 24;
const DATA: u8 = 0;
const HEADERS: u8 = 1;
const RST_STREAM: u8 = 3;
const SETTINGS: u8 = 4;
const PING: u8 = 6;
const GOAWAY: u8 = 7;
const WINDOW_UPDATE: u8 = 8;
const CONTINUATION: u8 = 9;
const END_STREAM: u8 = 1;
const ACK: u8 = 1;
const END_HEADERS: u8 = 4;
const STATUS_200: u8 = 0x88;
const INITIAL_WINDOW_SIZE: u16 = 4;
const HOSTILE_RATE: u64 = 200;
const HEAD_LIMIT_BYTES: usize = 1 << 20;
const MAX_RSS_GROWTH_KIB: u64 = 256 * 1024;
const MAX_EXTRA_CPUS: f64 = 1.5;
const SLACK: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Attack {
    None,
    PingFlood,
    SettingsFlood,
    EmptyData,
    EmptyContinuation,
    GrowingContinuation,
    RapidReset,
    ZeroWindow,
    WindowFlood,
    HugeHeaders,
    SlowBody,
    GoawayLoop,
    DataFlood,
}

const ATTACKS: [(&str, Attack); 13] = [
    ("none", Attack::None),
    ("ping", Attack::PingFlood),
    ("settings", Attack::SettingsFlood),
    ("empty-data", Attack::EmptyData),
    ("empty-continuation", Attack::EmptyContinuation),
    ("growing-continuation", Attack::GrowingContinuation),
    ("rapid-reset", Attack::RapidReset),
    ("zero-window", Attack::ZeroWindow),
    ("window-flood", Attack::WindowFlood),
    ("huge-headers", Attack::HugeHeaders),
    ("slow-body", Attack::SlowBody),
    ("goaway-loop", Attack::GoawayLoop),
    ("data-flood", Attack::DataFlood),
];

fn attack_named(name: &str) -> Attack {
    ATTACKS.iter().find(|(held, _)| *held == name).map_or_else(|| panic!("unknown attack {name}"), |(_, attack)| *attack)
}

pub fn text<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

pub fn frame(kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes()[1..]);
    out.extend_from_slice(&[kind, flags]);
    out.extend_from_slice(&stream.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn settings(attack: Attack) -> Vec<u8> {
    let mut payload = Vec::new();
    if attack == Attack::ZeroWindow {
        payload.extend_from_slice(&INITIAL_WINDOW_SIZE.to_be_bytes());
        payload.extend_from_slice(&0u32.to_be_bytes());
    }
    frame(SETTINGS, 0, 0, &payload)
}

pub fn answer(stream: u32) -> Vec<u8> {
    let mut out = frame(HEADERS, END_HEADERS, stream, &[STATUS_200]);
    out.extend(frame(DATA, END_STREAM, stream, b"ok"));
    out
}

fn huge_literal() -> Vec<u8> {
    let mut block = vec![0x00, 0x7f, 0x80, 0x80, 0x04];
    block.extend(std::iter::repeat_n(b'x', 1 << 16));
    block.extend_from_slice(&[0x7f, 0xff, 0xff, 0x03]);
    block.extend(std::iter::repeat_n(b'y', HEAD_LIMIT_BYTES));
    block
}

pub type Writer = WriteHalf<TlsStream<TcpStream>>;
pub type Reader = ReadHalf<TlsStream<TcpStream>>;

pub async fn next_frame(reader: &mut Reader) -> Option<(u8, u8, u32, Vec<u8>)> {
    let mut head = [0u8; 9];
    reader.read_exact(&mut head).await.ok()?;
    let length = u32::from_be_bytes([0, head[0], head[1], head[2]]) as usize;
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload).await.ok()?;
    let stream = u32::from_be_bytes([head[5], head[6], head[7], head[8]]) & 0x7fff_ffff;
    Some((head[3], head[4], stream, payload))
}

async fn flood(mut writer: Writer, frames: Vec<u8>) {
    while writer.write_all(&frames).await.is_ok() {}
}

async fn drain(mut reader: Reader) {
    let mut sink = vec![0u8; 1 << 16];
    while reader.read(&mut sink).await.is_ok_and(|n| n > 0) {}
}

async fn on_first_request(attack: Attack, reader: Reader, mut writer: Writer, stream: u32) {
    let empty = frame(DATA, 0, stream, &[]);
    match attack {
        Attack::PingFlood => flood(writer, frame(PING, 0, 0, &[7; 8])).await,
        Attack::SettingsFlood => flood(writer, frame(SETTINGS, 0, 0, &[])).await,
        Attack::EmptyData => {
            tokio::spawn(drain(reader));
            drop(writer.write_all(&frame(HEADERS, END_HEADERS, stream, &[STATUS_200])).await);
            flood(writer, empty).await;
        }
        Attack::EmptyContinuation | Attack::GrowingContinuation => {
            tokio::spawn(drain(reader));
            drop(writer.write_all(&frame(HEADERS, 0, stream, &[STATUS_200])).await);
            let chunk = if attack == Attack::EmptyContinuation { vec![] } else { vec![0x40; 16_384] };
            flood(writer, frame(CONTINUATION, 0, stream, &chunk)).await;
        }
        Attack::WindowFlood => {
            tokio::spawn(drain(reader));
            let mut both = frame(WINDOW_UPDATE, 0, 0, &1u32.to_be_bytes());
            both.extend(frame(WINDOW_UPDATE, 0, stream, &1u32.to_be_bytes()));
            flood(writer, both).await;
        }
        Attack::HugeHeaders => {
            tokio::spawn(drain(reader));
            let block = huge_literal();
            let mut chunks = block.chunks(16_384);
            let first = chunks.next().unwrap_or_default();
            drop(writer.write_all(&frame(HEADERS, 0, stream, first)).await);
            for chunk in chunks {
                drop(writer.write_all(&frame(CONTINUATION, 0, stream, chunk)).await);
            }
            drop(writer.write_all(&frame(CONTINUATION, END_HEADERS, stream, &[])).await);
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
        Attack::SlowBody => {
            tokio::spawn(drain(reader));
            drop(writer.write_all(&frame(HEADERS, END_HEADERS, stream, &[STATUS_200])).await);
            let drip = frame(DATA, 0, stream, b"x");
            while writer.write_all(&drip).await.is_ok() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        Attack::DataFlood => {
            tokio::spawn(drain(reader));
            drop(writer.write_all(&frame(HEADERS, END_HEADERS, stream, &[STATUS_200])).await);
            flood(writer, frame(DATA, 0, stream, &[0x61; 16_384])).await;
        }
        Attack::ZeroWindow => {
            drain(reader).await;
            drop(writer);
        }
        Attack::None | Attack::RapidReset | Attack::GoawayLoop => {}
    }
}

async fn serve_requests(attack: Attack, mut reader: Reader, mut writer: Writer) {
    while let Some((kind, flags, stream, _)) = next_frame(&mut reader).await {
        let reply = match (kind, attack) {
            (SETTINGS, _) if flags & ACK == 0 => frame(SETTINGS, ACK, 0, &[]),
            (HEADERS, Attack::None) => answer(stream),
            (HEADERS, Attack::RapidReset) => frame(RST_STREAM, 0, stream, &8u32.to_be_bytes()),
            (HEADERS, Attack::GoawayLoop) => {
                let mut payload = 0u32.to_be_bytes().to_vec();
                payload.extend_from_slice(&0u32.to_be_bytes());
                drop(writer.write_all(&frame(GOAWAY, 0, 0, &payload)).await);
                return;
            }
            (HEADERS, _) => return on_first_request(attack, reader, writer, stream).await,
            _ => continue,
        };
        if writer.write_all(&reply).await.is_err() || writer.flush().await.is_err() {
            return;
        }
    }
}

async fn connection(acceptor: TlsAcceptor, socket: TcpStream, attack: Attack) {
    drop(socket.set_nodelay(true));
    let Ok(mut stream) = acceptor.accept(socket).await else { return };
    let mut preface = [0u8; PREFACE_LEN];
    if stream.read_exact(&mut preface).await.is_err() {
        return;
    }
    if stream.write_all(&settings(attack)).await.is_err() {
        return;
    }
    let (reader, writer) = tokio::io::split(stream);
    serve_requests(attack, reader, writer).await;
}

async fn listen(acceptor: TlsAcceptor, port: u16, attack: Attack) {
    let listener = TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    loop {
        if let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(connection(acceptor.clone(), socket, attack));
        }
    }
}

pub fn acceptor(ca_path: &str) -> TlsAcceptor {
    drop(rustls::crypto::aws_lc_rs::default_provider().install_default());
    let issued = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).expect("certificate");
    let cert: CertificateDer<'static> = issued.cert.der().clone();
    std::fs::write(ca_path, &cert).expect("write certificate");
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(issued.signing_key.serialize_der()));
    let mut config = rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(vec![cert], key).expect("tls config");
    config.alpn_protocols = vec![b"h2".to_vec()];
    TlsAcceptor::from(Arc::new(config))
}

pub fn serve(args: &[String]) -> ! {
    let attack = attack_named(text(args, "--attack").unwrap_or("none"));
    let port: u16 = number(args, "--port", 8443);
    let threads: usize = number(args, "--origin-threads", 4);
    let runtime = Builder::new_multi_thread().worker_threads(threads).enable_all().build().expect("runtime");
    let tls = acceptor(text(args, "--ca").expect("--ca path"));
    runtime.spawn(listen(tls.clone(), port, Attack::None));
    runtime.block_on(listen(tls, port + 1, attack));
    unreachable!("the listener never returns")
}

struct Shared {
    start: Instant,
    healthy_ok: AtomicU64,
    healthy_failed: AtomicU64,
    outlived: AtomicU64,
    hostile: Mutex<HashMap<String, u64>>,
    first_error: Mutex<Option<String>>,
    failed_at: Mutex<Vec<(f64, f64)>>,
    before: Mutex<Histogram<u64>>,
    during: Mutex<Histogram<u64>>,
}

pub fn latency() -> Histogram<u64> {
    Histogram::new_with_bounds(1, 60_000_000, 3).expect("histogram")
}

async fn healthy_one(shared: Arc<Shared>, session: Session, url: Arc<str>, under_attack: bool, scheduled: Instant) {
    let failure = match session.get(&*url).send().await {
        Ok(response) if response.status() != 200 => Some(format!("status {}", response.status())),
        Ok(response) => match response.bytes().await {
            Ok(body) if &body[..] == b"ok" => None,
            Ok(_) => Some("wrong body".to_owned()),
            Err(error) => Some(format!("{error:?}")),
        },
        Err(error) => Some(format!("{error:?}")),
    };
    match failure {
        None => {
            shared.healthy_ok.fetch_add(1, Ordering::Relaxed);
        }
        Some(message) => {
            shared.healthy_failed.fetch_add(1, Ordering::Relaxed);
            shared.first_error.lock().unwrap().get_or_insert(message);
            let due = (scheduled - shared.start).as_secs_f64();
            shared.failed_at.lock().unwrap().push((due, scheduled.elapsed().as_secs_f64()));
        }
    }
    let held = if under_attack { &shared.during } else { &shared.before };
    held.lock().unwrap().saturating_record(scheduled.elapsed().as_micros() as u64);
}

async fn hostile_one(shared: Arc<Shared>, session: Session, url: Arc<str>, upload: bool, limit: Duration) {
    let began = Instant::now();
    let sent = if upload {
        session.post(&*url).body(Body::from(vec![0u8; 1 << 16])).send().await
    } else {
        session.get(&*url).send().await
    };
    let outcome = match sent {
        Ok(response) => response.bytes().await.map(|_| ()),
        Err(error) => Err(error),
    };
    if began.elapsed() > limit + SLACK {
        shared.outlived.fetch_add(1, Ordering::Relaxed);
    }
    let label = outcome.map_or_else(|e| e.kind().as_str().to_owned(), |()| "ok".to_owned());
    *shared.hostile.lock().unwrap().entry(label).or_insert(0) += 1;
}

pub async fn pace<F: FnMut(Instant)>(start: Instant, rate: u64, from: f64, to: f64, mut send: F) {
    let mut sent = 0u64;
    loop {
        let elapsed = start.elapsed().as_secs_f64();
        if elapsed >= to {
            return;
        }
        if elapsed >= from {
            let due = ((elapsed - from) * rate as f64) as u64;
            while sent < due {
                sent += 1;
                send(start + Duration::from_secs_f64(from + sent as f64 / rate as f64));
            }
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

pub fn rss_kib() -> u64 {
    let text = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    text.split_whitespace().nth(1).and_then(|p| p.parse::<u64>().ok()).unwrap_or(0) * 4
}

pub fn cpu_seconds() -> f64 {
    let text = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields: Vec<&str> = text.rsplit(')').next().unwrap_or("").split_whitespace().collect();
    [11, 12].iter().filter_map(|&i| fields.get(i)?.parse::<f64>().ok()).sum::<f64>() / 100.0
}

struct Usage {
    cpu_before: f64,
    cpu_during: f64,
    rss_at_attack_kib: u64,
    rss_peak_kib: u64,
}

async fn watch(start: Instant, quiet: f64, seconds: f64) -> Usage {
    tokio::time::sleep(Duration::from_secs_f64(quiet)).await;
    let (cpu_at_attack, rss_at_attack_kib) = (cpu_seconds(), rss_kib());
    let mut rss_peak_kib = rss_at_attack_kib;
    while start.elapsed().as_secs_f64() < seconds {
        tokio::time::sleep(Duration::from_millis(250)).await;
        rss_peak_kib = rss_peak_kib.max(rss_kib());
    }
    Usage {
        cpu_before: cpu_at_attack / quiet,
        cpu_during: (cpu_seconds() - cpu_at_attack) / (seconds - quiet),
        rss_at_attack_kib,
        rss_peak_kib,
    }
}

fn judge(shared: &Shared, usage: &Usage, ratio: f64, verdict: &Verdict) {
    let failed = shared.healthy_failed.load(Ordering::Relaxed);
    if failed > 0 {
        let first = shared.first_error.lock().unwrap().clone().unwrap_or_default();
        verdict.fail(format!("isolation: {failed} healthy requests failed, first: {first}"));
    }
    let outlived = shared.outlived.load(Ordering::Relaxed);
    if outlived > 0 {
        verdict.fail(format!("deadline: {outlived} hostile requests outlived their deadline"));
    }
    let (before, during) = (shared.before.lock().unwrap().value_at_quantile(0.99), shared.during.lock().unwrap().value_at_quantile(0.99));
    if during as f64 > before as f64 * ratio {
        verdict.fail(format!("isolation: healthy p99 {during} us under attack, {before} us before, ratio limit {ratio}"));
    }
    if usage.rss_peak_kib > usage.rss_at_attack_kib + MAX_RSS_GROWTH_KIB {
        verdict.fail(format!("memory: RSS {} KiB under attack, {} KiB before", usage.rss_peak_kib, usage.rss_at_attack_kib));
    }
    if usage.cpu_during > usage.cpu_before + MAX_EXTRA_CPUS {
        verdict.fail(format!("cpu: {:.2} logical CPUs under attack, {:.2} before", usage.cpu_during, usage.cpu_before));
    }
}

pub fn run(options: &Options, args: &[String], verdict: &Verdict) -> Value {
    let attack = attack_named(text(args, "--attack").unwrap_or("none"));
    let port: u16 = number(args, "--port", 8443);
    let rate: u64 = number(args, "--rate", 20_000);
    let limit = Duration::from_millis(number(args, "--deadline-ms", 2000));
    let ratio: f64 = number(args, "--p99-ratio", 1.5);
    let seconds = options.seconds as f64;
    let quiet = seconds / 3.0;
    let ca = std::fs::read(text(args, "--ca").expect("--ca path")).expect("read certificate");
    let shared = Arc::new(Shared {
        start: Instant::now(),
        healthy_ok: AtomicU64::new(0),
        healthy_failed: AtomicU64::new(0),
        outlived: AtomicU64::new(0),
        hostile: Mutex::default(),
        first_error: Mutex::default(),
        failed_at: Mutex::default(),
        before: Mutex::new(latency()),
        during: Mutex::new(latency()),
    });
    let runtime = Builder::new_multi_thread().enable_all().build().expect("runtime");
    let usage = runtime.block_on(async {
        let session = Session::builder()
            .protocol(Protocol::Http2)
            .tls(Tls::new().env_roots(false).system_roots(false).ca_der(ca))
            .retry(Retry::new(number(args, "--retries", 0)))
            .timeout(Timeout::new().total(limit))
            .build()
            .expect("session");
        let healthy_url: Arc<str> = format!("https://127.0.0.1:{port}/").into();
        let hostile_url: Arc<str> = format!("https://127.0.0.1:{}/", port + 1).into();
        let start = shared.start;
        let watching = tokio::spawn(watch(start, quiet, seconds));
        for _ in 0..number::<u64>(args, "--burst", 0) {
            tokio::spawn(healthy_one(Arc::clone(&shared), session.clone(), Arc::clone(&healthy_url), false, start));
        }
        let healthy = {
            let (shared, session) = (Arc::clone(&shared), session.clone());
            pace(start, rate, 0.0, seconds, move |at| {
                let under_attack = (at - start).as_secs_f64() >= quiet;
                tokio::spawn(healthy_one(Arc::clone(&shared), session.clone(), Arc::clone(&healthy_url), under_attack, at));
            })
        };
        let hostile = {
            let (shared, session) = (Arc::clone(&shared), session.clone());
            let upload = attack == Attack::ZeroWindow;
            pace(start, HOSTILE_RATE, quiet, seconds, move |_| {
                tokio::spawn(hostile_one(Arc::clone(&shared), session.clone(), Arc::clone(&hostile_url), upload, limit));
            })
        };
        tokio::join!(healthy, hostile);
        tokio::time::sleep(limit + SLACK + SLACK).await;
        watching.await.expect("watcher")
    });
    runtime.shutdown_timeout(Duration::from_secs(1));
    judge(&shared, &usage, ratio, verdict);
    let (before, during) = (shared.before.lock().unwrap(), shared.during.lock().unwrap());
    json!({
        "attack": text(args, "--attack").unwrap_or("none"),
        "healthy_ok": shared.healthy_ok.load(Ordering::Relaxed),
        "healthy_failed": shared.healthy_failed.load(Ordering::Relaxed),
        "failed_due_and_waited_seconds": shared.failed_at.lock().unwrap().iter().take(200).copied().collect::<Vec<(f64, f64)>>(),
        "first_error": shared.first_error.lock().unwrap().clone(),
        "healthy_p99_us": {"before": before.value_at_quantile(0.99), "during": during.value_at_quantile(0.99)},
        "hostile_outcomes": *shared.hostile.lock().unwrap(),
        "outlived_deadline": shared.outlived.load(Ordering::Relaxed),
        "cpu": {"before": usage.cpu_before, "during": usage.cpu_during},
        "rss_kib": {"at_attack": usage.rss_at_attack_kib, "peak": usage.rss_peak_kib},
    })
}
