use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hdrhistogram::Histogram;
use leyline::{Protocol, Retry, Session, Tls};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Builder;
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;

use crate::hostile::{
    Attack, Reader, Writer, acceptor, answer, frame, latency, next_frame, pace, settings, text,
};
use crate::plan::mix;
use crate::probe::{cpu_seconds, rss_kib};
use crate::{Options, Verdict, number};

const PREFACE_LEN: usize = 24;
const HEADERS: u8 = 1;
const SETTINGS: u8 = 4;
const GOAWAY: u8 = 7;
const ACK: u8 = 1;
const POST_INDEXED: u8 = 0x83;
const MAX_LAST_STREAM: u32 = 0x7FFF_FFFF;
const RECOVERY_LIMIT_SECONDS: usize = 5;

fn holds_stream(payload: &[u8]) -> bool {
    payload.iter().take(6).any(|byte| *byte == POST_INDEXED)
}

async fn answer_frames(
    mut reader: Reader,
    mut writer: Writer,
    mut restart: watch::Receiver<u64>,
    abrupt: bool,
) {
    loop {
        tokio::select! {
            read = next_frame(&mut reader) => {
                let Some((kind, flags, stream, payload)) = read else { return };
                let reply = match kind {
                    SETTINGS if flags & ACK == 0 => frame(SETTINGS, ACK, 0, &[]),
                    HEADERS if holds_stream(&payload) => continue,
                    HEADERS => answer(stream),
                    _ => continue,
                };
                if writer.write_all(&reply).await.is_err() || writer.flush().await.is_err() {
                    return;
                }
            }
            _ = restart.changed() => {
                if !abrupt {
                    let mut payload = MAX_LAST_STREAM.to_be_bytes().to_vec();
                    payload.extend_from_slice(&0u32.to_be_bytes());
                    drop(writer.write_all(&frame(GOAWAY, 0, 0, &payload)).await);
                    drop(writer.flush().await);
                }
                return;
            }
        }
    }
}

async fn connection(
    tls: TlsAcceptor,
    socket: TcpStream,
    restart: watch::Receiver<u64>,
    abrupt: bool,
) {
    drop(socket.set_nodelay(true));
    let Ok(mut stream) = tls.accept(socket).await else {
        return;
    };
    let mut preface = [0u8; PREFACE_LEN];
    if stream.read_exact(&mut preface).await.is_err()
        || stream.write_all(&settings(Attack::None)).await.is_err()
    {
        return;
    }
    let (reader, writer) = tokio::io::split(stream);
    answer_frames(reader, writer, restart, abrupt).await;
}

async fn listen(tls: TlsAcceptor, port: u16, restart: watch::Receiver<u64>, abrupt: bool) {
    let listener = TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    loop {
        if let Ok((socket, _)) = listener.accept().await {
            let mut current = restart.clone();
            current.borrow_and_update();
            tokio::spawn(connection(tls.clone(), socket, current, abrupt));
        }
    }
}

pub fn serve(args: &[String]) -> ! {
    let port: u16 = number(args, "--port", 8443);
    let hosts: u16 = number(args, "--hosts", 1);
    let abrupt = args.iter().any(|a| a == "--abrupt");
    let restart_at: f64 = number(args, "--restart-at", 0.0);
    let threads: usize = number(args, "--origin-threads", 4);
    let runtime = Builder::new_multi_thread()
        .worker_threads(threads)
        .enable_all()
        .build()
        .expect("runtime");
    let tls = acceptor(text(args, "--ca").expect("--ca path"));
    let (restart, restarted) = watch::channel(0u64);
    for offset in 0..hosts {
        runtime.spawn(listen(
            tls.clone(),
            port + offset,
            restarted.clone(),
            abrupt,
        ));
    }
    runtime.block_on(async move {
        if restart_at > 0.0 {
            tokio::time::sleep(Duration::from_secs_f64(restart_at)).await;
            let _ = restart.send(1);
        }
        std::future::pending::<()>().await;
    });
    unreachable!("the origin never returns")
}

fn session(args: &[String], ca: Vec<u8>, single_connection: bool) -> Session {
    let builder = Session::builder()
        .protocol(Protocol::Http2)
        .tls(Tls::new().env_roots(false).system_roots(false).ca_der(ca))
        .max_connections(number(args, "--max-connections", 4096))
        .retry(
            Retry::new(number(args, "--retries", 2))
                .backoff(Duration::from_millis(5), Duration::from_millis(50)),
        );
    let builder = if single_connection {
        builder.max_connections_per_host(1)
    } else {
        builder
    };
    builder.build().expect("session")
}

struct Seconds {
    start: Instant,
    latency: Vec<std::sync::Mutex<Histogram<u64>>>,
    failed: AtomicU64,
    first_error: std::sync::Mutex<Option<String>>,
    deadline: Duration,
}

impl Seconds {
    fn new(seconds: usize, deadline: Duration) -> Arc<Self> {
        Arc::new(Self {
            start: Instant::now(),
            latency: (0..seconds + 10)
                .map(|_| std::sync::Mutex::new(latency()))
                .collect(),
            failed: AtomicU64::new(0),
            first_error: std::sync::Mutex::default(),
            deadline,
        })
    }

    fn p99_by_second(&self) -> Vec<u64> {
        self.latency
            .iter()
            .map(|h| h.lock().unwrap().value_at_quantile(0.99))
            .collect()
    }
}

async fn one(seconds: Arc<Seconds>, session: Session, url: String, scheduled: Instant) {
    let reply = session
        .get(url.as_str())
        .timeout(seconds.deadline)
        .send()
        .await;
    let failure = match reply {
        Ok(response) if response.status() != 200 => Some(format!("status {}", response.status())),
        Ok(response) => response.bytes().await.err().map(|error| error.to_string()),
        Err(error) => Some(format!("{:?} {error:?}", error.kind())),
    };
    if let Some(message) = failure {
        seconds.failed.fetch_add(1, Ordering::Relaxed);
        seconds.first_error.lock().unwrap().get_or_insert(message);
    }
    let at = (scheduled - seconds.start).as_secs() as usize;
    if let Some(slot) = seconds.latency.get(at) {
        slot.lock()
            .unwrap()
            .saturating_record(scheduled.elapsed().as_micros() as u64);
    }
}

fn steady_p99(by_second: &[u64], from: usize, to: usize) -> u64 {
    let mut window: Vec<u64> = by_second[from..to].to_vec();
    window.sort_unstable();
    window[window.len() / 2]
}

pub fn run_reconnect(options: &Options, args: &[String], verdict: &Verdict) -> Value {
    let port: u16 = number(args, "--port", 8443);
    let hosts: u64 = number(args, "--hosts", 2000);
    let rate: u64 = number(args, "--rate", 100_000);
    let total = options.seconds as usize;
    let restart_at = total / 2;
    let ca = std::fs::read(text(args, "--ca").expect("--ca path")).expect("read certificate");
    let seconds = Seconds::new(
        total,
        Duration::from_millis(number(args, "--deadline-ms", 5000)),
    );
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let session = session(args, ca, false);
        let start = seconds.start;
        let (shared, mut id) = (Arc::clone(&seconds), 0u64);
        pace(start, rate, 0.0, total as f64, move |at| {
            id += 1;
            let host = port as u64 + mix(id) % hosts;
            tokio::spawn(one(
                Arc::clone(&shared),
                session.clone(),
                format!("https://127.0.0.1:{host}/"),
                at,
            ));
        })
        .await;
        tokio::time::sleep(Duration::from_secs(6)).await;
    });
    runtime.shutdown_timeout(Duration::from_secs(1));
    let by_second = seconds.p99_by_second();
    let settled = steady_p99(&by_second, 2, restart_at);
    let limit = (settled as f64 * number(args, "--p99-ratio", 1.5)) as u64;
    let recovery = by_second[restart_at - 2..total]
        .iter()
        .filter(|p99| **p99 > limit)
        .count();
    let failed = seconds.failed.load(Ordering::Relaxed);
    if failed > 0 {
        let first = seconds
            .first_error
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_default();
        verdict.fail(format!(
            "failures: {failed} requests failed across the restart, first: {first}"
        ));
    }
    if recovery > RECOVERY_LIMIT_SECONDS {
        verdict.fail(format!(
            "recovery: {recovery} s over {limit} us, limit {RECOVERY_LIMIT_SECONDS} s"
        ));
    }
    json!({
        "hosts": hosts, "offered_rps": rate, "restart_at_second": restart_at,
        "failed": failed, "seconds_over_limit": recovery, "settled_p99_us": settled,
        "peak_p99_us": by_second[restart_at - 2..total].iter().max(),
        "p99_by_second_us": by_second, "rss_kib": rss_kib(), "cpu_seconds": cpu_seconds(),
    })
}

pub fn run_idle(options: &Options, args: &[String], verdict: &Verdict) -> Value {
    let port: u16 = number(args, "--port", 8443);
    let rate: u64 = number(args, "--rate", 20_000);
    let streams: u64 = number(args, "--streams", 100_000);
    let total = options.seconds as usize;
    let (opening, settled_from) = (total / 3, 2 * total / 3);
    let ca = std::fs::read(text(args, "--ca").expect("--ca path")).expect("read certificate");
    let seconds = Seconds::new(
        total,
        Duration::from_millis(number(args, "--deadline-ms", 5000)),
    );
    let opened = Arc::new(AtomicU64::new(0));
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (rss_before, rss_after, cpu_before, cpu_after) = runtime.block_on(async {
        let session = session(args, ca, true);
        let start = seconds.start;
        let url = format!("https://127.0.0.1:{port}/");
        let busy = {
            let (shared, session, url) = (Arc::clone(&seconds), session.clone(), url.clone());
            pace(start, rate, 0.0, total as f64, move |at| {
                tokio::spawn(one(Arc::clone(&shared), session.clone(), url.clone(), at));
            })
        };
        let quiet = {
            let (session, url, opened) = (session.clone(), url.clone(), Arc::clone(&opened));
            let open_rate = streams * 2 / opening.max(1) as u64;
            let window = opening as f64 + streams as f64 / open_rate as f64;
            pace(start, open_rate, opening as f64, window, move |_| {
                let (session, url, opened) = (session.clone(), url.clone(), Arc::clone(&opened));
                tokio::spawn(async move {
                    opened.fetch_add(1, Ordering::Relaxed);
                    drop(session.post(url.as_str()).send().await);
                });
            })
        };
        let usage = async {
            tokio::time::sleep(Duration::from_secs(opening as u64)).await;
            let before = (rss_kib(), cpu_seconds());
            tokio::time::sleep(Duration::from_secs((settled_from - opening) as u64)).await;
            let settled = (rss_kib(), cpu_seconds());
            tokio::time::sleep(Duration::from_secs((total - settled_from) as u64)).await;
            let cpu_rate = |from: f64, to: f64, span: usize| (to - from) / span as f64;
            (
                before.0,
                settled.0,
                before.1 / opening as f64,
                cpu_rate(settled.1, cpu_seconds(), total - settled_from),
            )
        };
        let ((), (), usage) = tokio::join!(busy, quiet, usage);
        usage
    });
    runtime.shutdown_background();
    let by_second = seconds.p99_by_second();
    let (before, during) = (
        steady_p99(&by_second, 1, opening),
        steady_p99(&by_second, settled_from, total),
    );
    let ratio: f64 = number(args, "--p99-ratio", 1.5);
    if during as f64 > before as f64 * ratio {
        verdict.fail(format!("isolation: busy p99 {during} us with idle streams, {before} us before, ratio limit {ratio}"));
    }
    let failed = seconds.failed.load(Ordering::Relaxed);
    if failed > 0 {
        let first = seconds
            .first_error
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_default();
        verdict.fail(format!(
            "failures: {failed} busy requests failed, first: {first}"
        ));
    }
    let open = opened.load(Ordering::Relaxed);
    json!({
        "idle_streams": open, "offered_rps": rate, "failed": failed,
        "busy_p99_us": {"before": before, "during": during},
        "rss_kib": {"before": rss_before, "with_idle": rss_after},
        "kib_per_idle_stream": rss_after.saturating_sub(rss_before) as f64 / open.max(1) as f64,
        "cpu": {"before": cpu_before, "with_idle": cpu_after},
        "p99_by_second_us": by_second,
    })
}
