use std::io::{Read, Write};
use std::net::{IpAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hdrhistogram::Histogram;
use leyline::{HostLimits, Kind, Retry, Session, Timeout};
use serde_json::{Value, json};
use tokio::runtime::Builder;

use tokio::sync::Semaphore;

use crate::origin::{Capacity, Origin, Planner};
use crate::rival::Rival;
use crate::plan::{Fault, ID_HEADER, Planned, SIZE_HEADER, body_for, mix, stall, sticky_fault};
use crate::{Options, Verdict, number};

const SHARDS: usize = 16;
const MIN_REUSE: f64 = 0.999;
const MIN_ACHIEVED: f64 = 0.95;
const MAX_RSS_GROWTH: f64 = 1.05;
const MIN_RECOVERY: f64 = 0.9;
const DEADLINE_SLACK: Duration = Duration::from_secs(1);
const DEADLINE: Duration = Duration::from_secs(10);
const ADDRESS_BASE: u32 = 1 << 16;
const CONTROL_ATTEMPTS: usize = 20;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Shared,
    Cores,
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Warm = 0,
    Before = 1,
    Fault = 2,
    After = 3,
}

const PHASES: [(Phase, &str); 4] = [
    (Phase::Warm, "warm"),
    (Phase::Before, "before"),
    (Phase::Fault, "fault"),
    (Phase::After, "after"),
];

struct Layout {
    port: u16,
    hosts: u32,
    bad_first: u32,
    bad_count: u32,
    rate: f64,
    seconds: f64,
    shock: f64,
    overload: bool,
}

impl Layout {
    fn is_bad(&self, rank: u32) -> bool {
        rank >= self.bad_first && rank < self.bad_first + self.bad_count
    }

    fn due(&self, elapsed: f64) -> u64 {
        let (from, to) = (0.4 * self.seconds, 0.7 * self.seconds);
        let inside = (elapsed.min(to) - from).max(0.0);
        (self.rate * (elapsed + inside * (self.shock - 1.0))) as u64
    }

    fn scheduled(&self, sent: u64) -> f64 {
        let (from, to) = (0.4 * self.seconds, 0.7 * self.seconds);
        let plain = sent as f64 / self.rate;
        if plain <= from {
            return plain;
        }
        let inside = (sent as f64 - self.rate * from) / (self.rate * self.shock);
        if inside <= to - from { from + inside } else { plain - (to - from) * (self.shock - 1.0) }
    }

    fn phase_at(&self, elapsed: f64) -> Phase {
        match elapsed / self.seconds {
            f if f < 0.1 => Phase::Warm,
            f if f < 0.4 => Phase::Before,
            f if f < 0.7 => Phase::Fault,
            _ => Phase::After,
        }
    }
}

fn host_ip(rank: u32) -> u32 {
    (127 << 24) | (ADDRESS_BASE + rank)
}

fn rank_of(ip: IpAddr) -> Option<u32> {
    match ip {
        IpAddr::V4(v4) => (u32::from(v4) & 0x00ff_ffff).checked_sub(ADDRESS_BASE),
        IpAddr::V6(_) => None,
    }
}

pub fn serve_origin(args: &[String]) -> ! {
    let seed: u64 = number(args, "--seed", 1);
    let threads: usize = number(args, "--origin-threads", 8);
    let port: u16 = number(args, "--port", 8000);
    let first: u32 = number(args, "--bad-first", 20);
    let count: u32 = number(args, "--bad-count", 50);
    let stall_only = args.iter().any(|a| a == "--stall-only");
    let planner: Planner = Arc::new(move |id, ip| match rank_of(ip) {
        Some(rank) if rank >= first && rank < first + count && stall_only => stall(),
        Some(rank) if rank >= first && rank < first + count => sticky_fault(seed, id),
        _ => Planned { fault: Fault::None, sticky: false },
    });
    let origin = Origin::listen(threads, &format!("0.0.0.0:{port}"), planner);
    let permits: usize = number(args, "--capacity-permits", 0);
    if permits > 0 {
        *origin.state.capacity.lock().unwrap() = Some(Capacity {
            permits: Arc::new(Semaphore::new(permits)),
            service: Duration::from_micros(number(args, "--service-us", 1000)),
            queue_limit: number(args, "--queue-limit", usize::MAX),
            queued: Arc::default(),
        });
    }
    origin.state.close_every.store(number(args, "--close-every", 0), Ordering::Relaxed);
    origin.state.close_silently.store(args.iter().any(|a| a == "--close-silently"), Ordering::Relaxed);
    println!("READY {}", origin.url);
    loop {
        std::thread::park();
    }
}

fn control(port: u16, path: &str) -> (u64, u64, u64) {
    for _ in 0..CONTROL_ATTEMPTS {
        if let Ok(fields) = control_once(port, path) {
            return fields;
        }
    }
    panic!("origin control failed {CONTROL_ATTEMPTS} times");
}

fn control_once(port: u16, path: &str) -> std::io::Result<(u64, u64, u64)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(CONTROL_TIMEOUT))?;
    write!(stream, "GET {path} HTTP/1.1\r\nhost: control\r\nconnection: close\r\n\r\n")?;
    let mut text = String::new();
    stream.read_to_string(&mut text)?;
    let body = text.rsplit("\r\n\r\n").next().unwrap_or_default();
    let mut fields = body.split_whitespace().map(|f| f.parse().map_err(|_| std::io::Error::other("control field")));
    let mut next = || fields.next().unwrap_or_else(|| Err(std::io::Error::other("short control answer")));
    Ok((next()?, next()?, next()?))
}

struct Zipf(Vec<f64>);

impl Zipf {
    fn new(hosts: u32) -> Self {
        let mut total = 0.0;
        let mut cdf: Vec<f64> = (1..=hosts)
            .map(|rank| {
                total += 1.0 / f64::from(rank);
                total
            })
            .collect();
        for value in &mut cdf {
            *value /= total;
        }
        Self(cdf)
    }

    fn pick(&self, roll: u64) -> u32 {
        let unit = (roll >> 11) as f64 / (1u64 << 53) as f64;
        self.0.partition_point(|&p| p < unit).min(self.0.len() - 1) as u32
    }
}

fn size_of(roll: u64) -> usize {
    match roll % 1000 {
        0 => 1 << 20,
        1..=10 => 64 << 10,
        _ => 1 << 10,
    }
}

type Latency = Mutex<Histogram<u64>>;

fn latency() -> Latency {
    Mutex::new(Histogram::new_with_bounds(1, 60_000_000, 3).expect("histogram"))
}

struct Shared {
    layout: Layout,
    zipf: Zipf,
    seed: u64,
    start: Instant,
    next_id: AtomicU64,
    in_flight: AtomicU64,
    completed: AtomicU64,
    bad_errors: AtomicU64,
    healthy: [Vec<Latency>; 4],
    bad: [Vec<Latency>; 4],
    good: [AtomicU64; 4],
    visited: Vec<AtomicBool>,
    cold: AtomicU64,
    cold_shed: AtomicU64,
    outlived: AtomicU64,
    limit: Duration,
    rival: Option<Rival>,
    seconds: Vec<Latency>,
    failures: Mutex<Vec<String>>,
    wrong: AtomicU64,
}

impl Shared {
    fn fail(&self, message: String) {
        if self.wrong.fetch_add(1, Ordering::Relaxed) < 40 {
            self.failures.lock().unwrap().push(message);
        }
    }

    fn record(&self, phase: Phase, bad: bool, id: u64, micros: u64) {
        let groups = if bad { &self.bad } else { &self.healthy };
        let shard = &groups[phase as usize][(id as usize) % SHARDS];
        shard.lock().unwrap().saturating_record(micros);
    }

    fn record_second(&self, scheduled: Instant, micros: u64) {
        let at = (scheduled - self.start).as_secs() as usize;
        if let Some(slot) = self.seconds.get(at) {
            slot.lock().unwrap().saturating_record(micros);
        }
    }
}

enum Reply {
    Right,
    Wrong,
    Refused,
}

struct Failed {
    overloaded: bool,
    message: String,
}

impl From<leyline::Error> for Failed {
    fn from(error: leyline::Error) -> Self {
        Self { overloaded: error.kind() == Kind::Overloaded, message: format!("kind={:?} {error}", error.kind()) }
    }
}

impl From<String> for Failed {
    fn from(message: String) -> Self {
        Self { overloaded: false, message }
    }
}

struct Call<'a> {
    url: &'a str,
    ip: std::net::Ipv4Addr,
    port: u16,
    id: u64,
    size: usize,
}

async fn fetch(session: &Session, rival: Option<&Rival>, call: &Call<'_>) -> Result<(u16, Vec<u8>), Failed> {
    if let Some(rival) = rival {
        let answer = rival.get(call.ip, call.port, call.id, call.size).await?;
        return Ok((answer.status, answer.body.to_vec()));
    }
    let response = session
        .get(call.url)
        .header(ID_HEADER, call.id.to_string())
        .header(SIZE_HEADER, call.size.to_string())
        .send()
        .await?;
    let status = response.status().as_u16();
    Ok((status, response.bytes().await?.to_vec()))
}

async fn exchange(session: &Session, rival: Option<&Rival>, call: &Call<'_>, bad: bool, shed: bool) -> Result<Reply, Failed> {
    let (id, size) = (call.id, call.size);
    let (status, body) = fetch(session, rival, call).await?;
    let expected = body_for(id);
    if status != 200 {
        return Ok(if bad || (shed && status == 503) { Reply::Refused } else { Reply::Wrong });
    }
    let whole = body.len() == size.max(expected.len()) || bad;
    Ok(if body.starts_with(expected.as_bytes()) && whole { Reply::Right } else { Reply::Wrong })
}

async fn one(shared: Arc<Shared>, session: Session, id: u64, scheduled: Instant) {
    let roll = mix(shared.seed ^ mix(id));
    let rank = shared.zipf.pick(roll);
    let bad = shared.layout.is_bad(rank);
    let ip = std::net::Ipv4Addr::from(host_ip(rank));
    let url = format!("http://{ip}:{}/", shared.layout.port);
    let phase = shared.layout.phase_at((scheduled - shared.start).as_secs_f64());
    if !shared.visited[rank as usize].swap(true, Ordering::Relaxed) {
        shared.cold.fetch_add(1, Ordering::Relaxed);
    }
    let polled = Instant::now();
    let result = {
        let call = Call { url: &url, ip, port: shared.layout.port, id, size: size_of(roll >> 7) };
        exchange(&session, shared.rival.as_ref(), &call, bad, shared.layout.overload).await
    };
    if polled.elapsed() > shared.limit + DEADLINE_SLACK {
        shared.outlived.fetch_add(1, Ordering::Relaxed);
    }
    let micros = scheduled.elapsed().as_micros() as u64;
    shared.record(phase, bad, id, micros);
    if !bad {
        shared.record_second(scheduled, micros);
    }
    match result {
        Ok(Reply::Right) => {
            shared.good[phase as usize].fetch_add(1, Ordering::Relaxed);
        }
        Ok(Reply::Refused) => {}
        Ok(Reply::Wrong) => shared.fail(format!("cross-talk or corrupt body: id={id} host={ip}")),
        Err(_) if bad || shared.layout.overload => {
            shared.bad_errors.fetch_add(1, Ordering::Relaxed);
        }
        Err(error) if error.overloaded && phase == Phase::Warm => {
            shared.cold_shed.fetch_add(1, Ordering::Relaxed);
        }
        Err(error) => shared.fail(format!("healthy host failed: id={id} host={ip} {}", error.message)),
    }
    shared.completed.fetch_add(1, Ordering::Relaxed);
    shared.in_flight.fetch_sub(1, Ordering::Relaxed);
}

async fn pacer(shared: Arc<Shared>, session: Session, share: f64) {
    let mut sent = 0u64;
    loop {
        let elapsed = shared.start.elapsed().as_secs_f64();
        if elapsed >= shared.layout.seconds {
            break;
        }
        let due = (shared.layout.due(elapsed) as f64 * share) as u64;
        while sent < due {
            sent += 1;
            let at = shared.layout.scheduled((sent as f64 / share) as u64);
            let scheduled = shared.start + Duration::from_secs_f64(at);
            let id = shared.next_id.fetch_add(1, Ordering::Relaxed);
            shared.in_flight.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(one(Arc::clone(&shared), session.clone(), id, scheduled));
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    while shared.in_flight.load(Ordering::Relaxed) > 0
        && shared.start.elapsed().as_secs_f64() < shared.layout.seconds + DEADLINE.as_secs_f64()
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn retry_policy(args: &[String]) -> Retry {
    let backoff = (Duration::from_millis(number(args, "--backoff-min-ms", 5)), Duration::from_millis(number(args, "--backoff-max-ms", 20)));
    let retry = Retry::new(number(args, "--retries", 1)).backoff(backoff.0, backoff.1);
    match number(args, "--retry-budget", 0.0) {
        0.0 => retry,
        ratio => retry.budget(ratio),
    }
}

fn builder(args: &[String]) -> leyline::SessionBuilder {
    let mut builder = Session::builder().retry(retry_policy(args));
    match args.iter().position(|a| a == "--max-pending").and_then(|i| args.get(i + 1)).map(String::as_str) {
        Some("none") => builder = builder.max_pending_per_host(None),
        Some(_) => builder = builder.max_pending_per_host(number::<usize>(args, "--max-pending", 0)),
        None => {}
    }
    let builder = builder
        .max_connections(number(args, "--max-connections", 2048))
        .host_limits(total_limit(args))
        .timeout(Timeout::new().total(Duration::from_millis(number(args, "--deadline-ms", DEADLINE.as_millis() as u64))));
    match number::<u64>(args, "--max-lifetime-ms", 0) {
        0 => builder,
        lifetime => builder.pool_max_lifetime(Duration::from_millis(lifetime)),
    }
}

fn drive(mode: Mode, shared: &Arc<Shared>, args: &[String]) {
    match mode {
        Mode::Shared => {
            let runtime = Builder::new_multi_thread().enable_all().build().expect("runtime");
            let session = runtime.block_on(async { builder(args).build() }).expect("session");
            runtime.block_on(pacer(Arc::clone(shared), session, 1.0));
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
        Mode::Cores => {
            let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
            builder(args)
                .run_per_core(|session| pacer(Arc::clone(shared), session, 1.0 / cores as f64))
                .expect("per-core run");
        }
    }
}

fn rss_kib() -> u64 {
    let text = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    text.split_whitespace().nth(1).and_then(|p| p.parse::<u64>().ok()).unwrap_or(0) * 4
}

fn cpu_ticks() -> u64 {
    let text = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields: Vec<&str> = text.rsplit(')').next().unwrap_or("").split_whitespace().collect();
    [11, 12].iter().filter_map(|&i| fields.get(i)?.parse::<u64>().ok()).sum()
}

fn open_fds() -> u64 {
    std::fs::read_dir("/proc/self/fd").map_or(0, |d| d.count() as u64)
}

fn tcp_counter(name: &str) -> u64 {
    let text = std::fs::read_to_string("/proc/net/netstat").unwrap_or_default();
    let mut lines = text.lines().filter(|line| line.starts_with("TcpExt:"));
    let (Some(names), Some(values)) = (lines.next(), lines.next()) else { return 0 };
    names.split_whitespace().zip(values.split_whitespace())
        .find(|(held, _)| *held == name)
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0)
}

struct Timeline {
    stop: AtomicBool,
    fds: AtomicU64,
    samples: Mutex<Vec<(f64, u64, u64, u64)>>,
    windows: Mutex<Vec<(u64, u64)>>,
}

fn conductor(shared: &Arc<Shared>, timeline: &Arc<Timeline>) -> std::thread::JoinHandle<()> {
    let (shared, timeline) = (Arc::clone(shared), Arc::clone(timeline));
    let counting = Arc::clone(&timeline);
    std::thread::spawn(move || {
        while !counting.stop.load(Ordering::Relaxed) {
            counting.fds.store(open_fds(), Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(250));
        }
    });
    std::thread::spawn(move || {
        let (port, seconds) = (shared.layout.port, shared.layout.seconds);
        let marks = [(0.1, None), (0.4, Some("/__faults/on")), (0.7, Some("/__faults/off")), (1.0, None)];
        let mut next = 0;
        while !timeline.stop.load(Ordering::Relaxed) {
            let at = shared.start.elapsed().as_secs_f64();
            timeline.samples.lock().unwrap().push((at, rss_kib(), timeline.fds.load(Ordering::Relaxed), cpu_ticks()));
            while next < marks.len() && at >= marks[next].0 * seconds {
                let (accepted, served, _) = control(port, marks[next].1.unwrap_or("/__stats"));
                let reconnects = accepted.saturating_sub(shared.cold.load(Ordering::Relaxed));
                timeline.windows.lock().unwrap().push((reconnects, served));
                next += 1;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    })
}

fn merged(shards: &[Latency]) -> Histogram<u64> {
    let mut all = Histogram::new_with_bounds(1, 60_000_000, 3).expect("histogram");
    for shard in shards {
        all.add(&*shard.lock().unwrap()).expect("merge");
    }
    all
}

fn summary(h: &Histogram<u64>) -> Value {
    json!({"count": h.len(), "p50_us": h.value_at_quantile(0.5), "p99_us": h.value_at_quantile(0.99),
           "p999_us": h.value_at_quantile(0.999), "max_us": h.max()})
}

fn reuse(windows: &[(u64, u64)], from: usize, to: usize) -> Option<f64> {
    let (a, b) = (windows.get(from)?, windows.get(to)?);
    let served = b.1.checked_sub(a.1)?;
    (served > 0).then(|| 1.0 - (b.0 - a.0) as f64 / served as f64)
}

fn tail_growth(samples: &[(f64, u64, u64, u64)], seconds: f64) -> Option<f64> {
    let mean = |from: f64, to: f64| {
        let picked: Vec<u64> = samples
            .iter()
            .filter(|s| s.0 >= from * seconds && s.0 < to * seconds)
            .map(|s| s.1)
            .collect();
        (!picked.is_empty()).then(|| picked.iter().sum::<u64>() as f64 / picked.len() as f64)
    };
    Some(mean(0.85, 1.0)? / mean(0.75, 0.85)?)
}

fn total_limit(args: &[String]) -> HostLimits {
    match number::<usize>(args, "--max-total", 0) {
        0 => HostLimits::new(),
        total => HostLimits::new()
            .max_total_in_flight(total)
            .max_total_pending(number(args, "--max-total-pending", usize::MAX)),
    }
}

struct Limits {
    ratio: f64,
    min_reuse: f64,
    min_goodput: f64,
    max_rss_kib: u64,
    max_rss_growth: f64,
}

fn judge_goodput(shared: &Shared, limits: &Limits, verdict: &Verdict) {
    let window = 0.3 * shared.layout.seconds;
    for (phase, name) in [(Phase::Fault, "during load"), (Phase::After, "after load")] {
        let rate = shared.good[phase as usize].load(Ordering::Relaxed) as f64 / window;
        if rate == 0.0 || rate < limits.min_goodput {
            verdict.fail(format!("goodput: {rate:.0} good req/s {name}, floor {:.0}", limits.min_goodput));
        }
    }
}

fn judge_shock(shared: &Shared, seconds: [f64; 2], verdict: &Verdict) {
    let [before, after] = [Phase::Before, Phase::After].map(|p| shared.good[p as usize].load(Ordering::Relaxed) as f64);
    let (before_rate, after_rate) = (before / seconds[0], after / seconds[1]);
    if after_rate < before_rate * MIN_RECOVERY {
        verdict.fail(format!("recovery: {after_rate:.0} good req/s after the shock, {before_rate:.0} before it"));
    }
}

fn judge(shared: &Shared, limits: &Limits, achieved: f64, before_reuse: Option<f64>, growth: Option<f64>, rss_peak: u64, verdict: &Verdict) {
    let ratio = limits.ratio;
    let layout = &shared.layout;
    for message in shared.failures.lock().unwrap().iter() {
        verdict.fail(message.clone());
    }
    let outlived = shared.outlived.load(Ordering::Relaxed);
    if outlived > 0 {
        verdict.fail(format!("deadline: {outlived} requests outlived their deadline"));
    }
    if limits.max_rss_kib > 0 && rss_peak > limits.max_rss_kib {
        verdict.fail(format!("memory: RSS peak {rss_peak} KiB, limit {} KiB", limits.max_rss_kib));
    }
    if layout.overload {
        judge_goodput(shared, limits, verdict);
        judge_shock(shared, [0.3 * layout.seconds, 0.3 * layout.seconds], verdict);
        return;
    }
    if achieved < layout.rate * MIN_ACHIEVED {
        verdict.fail(format!("rate: achieved {achieved:.0} req/s of {:.0} offered; this run says nothing about that rate", layout.rate));
    }
    if before_reuse.is_some_and(|r| r < limits.min_reuse) {
        verdict.fail(format!("reuse: {:.4} before faults, floor {}", before_reuse.unwrap_or(0.0), limits.min_reuse));
    }
    let before = merged(&shared.healthy[Phase::Before as usize]).value_at_quantile(0.99);
    let during = merged(&shared.healthy[Phase::Fault as usize]).value_at_quantile(0.99);
    if during as f64 > before as f64 * ratio {
        verdict.fail(format!("isolation: healthy p99 {during} us during faults, {before} us before, ratio limit {ratio}"));
    }
    let after = merged(&shared.healthy[Phase::After as usize]).value_at_quantile(0.99);
    if after as f64 > before as f64 * ratio {
        verdict.fail(format!("recovery: healthy p99 {after} us after faults, {before} us before, ratio limit {ratio}"));
    }
    if growth.is_some_and(|g| g > limits.max_rss_growth) {
        verdict.fail(format!("memory: RSS grew {:.3}x over the last 25% of the run", growth.unwrap_or(0.0)));
    }
}

pub fn run(options: &Options, args: &[String], verdict: &Verdict) -> Value {
    let mode = if args.iter().any(|a| a == "cores") { Mode::Cores } else { Mode::Shared };
    let layout = Layout {
        port: number(args, "--port", 8000),
        hosts: number(args, "--hosts", 2000),
        bad_first: number(args, "--bad-first", 20),
        bad_count: number(args, "--bad-count", 50),
        rate: number(args, "--rate", 100_000.0),
        seconds: options.seconds as f64,
        shock: number(args, "--shock", 1.0),
        overload: args.iter().any(|a| a == "--overload"),
    };
    let shards = || std::array::from_fn(|_| (0..SHARDS).map(|_| latency()).collect());
    let shared = Arc::new(Shared {
        zipf: Zipf::new(layout.hosts),
        layout,
        seed: options.seed,
        start: Instant::now(),
        next_id: AtomicU64::new(1),
        in_flight: AtomicU64::new(0),
        completed: AtomicU64::new(0),
        bad_errors: AtomicU64::new(0),
        healthy: shards(),
        bad: shards(),
        good: std::array::from_fn(|_| AtomicU64::new(0)),
        visited: (0..number(args, "--hosts", 2000)).map(|_| AtomicBool::new(false)).collect(),
        cold: AtomicU64::new(0),
        cold_shed: AtomicU64::new(0),
        outlived: AtomicU64::new(0),
        limit: Duration::from_millis(number(args, "--deadline-ms", DEADLINE.as_millis() as u64)),
        rival: args.iter().any(|a| a == "pingora").then(|| {
            let deadline = Duration::from_millis(number(args, "--deadline-ms", DEADLINE.as_millis() as u64));
            Rival::new(number(args, "--max-connections", 2048), deadline)
        }),
        seconds: (0..options.seconds as usize + 15).map(|_| latency()).collect(),
        failures: Mutex::default(),
        wrong: AtomicU64::new(0),
    });
    let overflows_at_start = tcp_counter("ListenOverflows");
    let retransmits_at_start = tcp_counter("TCPSynRetrans");
    let timeline = Arc::new(Timeline { stop: AtomicBool::new(false), fds: AtomicU64::new(0), samples: Mutex::default(), windows: Mutex::default() });
    let conducting = conductor(&shared, &timeline);
    drive(mode, &shared, args);
    let achieved = shared.completed.load(Ordering::Relaxed) as f64 / shared.layout.seconds;
    timeline.stop.store(true, Ordering::Relaxed);
    conducting.join().expect("conductor");
    let (samples, windows) = (timeline.samples.lock().unwrap(), timeline.windows.lock().unwrap());
    let before_reuse = reuse(&windows, 0, 1);
    let growth = tail_growth(&samples, shared.layout.seconds);
    let overflows = tcp_counter("ListenOverflows").saturating_sub(overflows_at_start);
    let syn_retransmits = tcp_counter("TCPSynRetrans").saturating_sub(retransmits_at_start);
    if overflows > 0 {
        verdict.fail(format!("rig: void run, {overflows} accept queue overflows on this machine"));
    }
    let limits = Limits {
        ratio: number(args, "--p99-ratio", 1.5),
        min_reuse: number(args, "--min-reuse", MIN_REUSE),
        min_goodput: number(args, "--min-goodput", 0.0),
        max_rss_kib: number::<u64>(args, "--max-rss-mib", 0) * 1024,
        max_rss_growth: number(args, "--max-rss-growth", MAX_RSS_GROWTH),
    };
    let rss_peak = samples.iter().map(|s| s.1).max().unwrap_or(0);
    let fds_peak = samples.iter().map(|s| s.2).max().unwrap_or(0);
    let max_fds: u64 = number(args, "--max-fds", u64::MAX);
    if fds_peak > max_fds {
        verdict.fail(format!("fds: {fds_peak} open at peak, limit {max_fds}"));
    }
    judge(&shared, &limits, achieved, before_reuse, growth, rss_peak, verdict);
    let groups: serde_json::Map<String, Value> = PHASES
        .iter()
        .map(|&(phase, name)| {
            let at = phase as usize;
            (name.to_owned(), json!({"healthy": summary(&merged(&shared.healthy[at])), "bad": summary(&merged(&shared.bad[at]))}))
        })
        .collect();
    json!({
        "mode": if shared.rival.is_some() { "pingora" } else if mode == Mode::Cores { "cores" } else { "shared" },
        "hosts": shared.layout.hosts, "bad_hosts": shared.layout.bad_count,
        "offered_rps": shared.layout.rate, "achieved_rps": achieved,
        "failures_total": shared.wrong.load(Ordering::Relaxed),
        "bad_host_errors": shared.bad_errors.load(Ordering::Relaxed),
        "reuse_before_faults": before_reuse, "reuse_after_faults": reuse(&windows, 2, 3),
        "reconnects_served_marks": *windows,
        "rss_peak_kib": samples.iter().map(|s| s.1).max(), "fds_peak": samples.iter().map(|s| s.2).max(),
        "rss_tail_growth": growth,
        "shock": shared.layout.shock,
        "listen_overflows": overflows,
        "syn_retransmits": syn_retransmits,
        "cold_start_shed": shared.cold_shed.load(Ordering::Relaxed),
        "outlived_deadline": shared.outlived.load(Ordering::Relaxed),
        "good": PHASES.iter().map(|&(p, name)| (name.to_owned(), json!(shared.good[p as usize].load(Ordering::Relaxed)))).collect::<serde_json::Map<String, Value>>(),
        "healthy_p99_by_second_us": shared.seconds.iter().map(|s| s.lock().unwrap().value_at_quantile(0.99)).collect::<Vec<u64>>(),
        "fds_by_second": samples.iter().map(|s| s.2).step_by(4).collect::<Vec<u64>>(),
        "rss_kib_by_second": samples.iter().map(|s| s.1).step_by(4).collect::<Vec<u64>>(),
        "cpu_by_second": samples.windows(2).map(|w| {
            let span = w[1].0 - w[0].0;
            if span > 0.0 { (w[1].3.saturating_sub(w[0].3) as f64 / 100.0 / span * 10.0).round() / 10.0 } else { 0.0 }
        }).step_by(4).collect::<Vec<f64>>(),
        "latency": groups,
    })
}
