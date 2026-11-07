use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::Semaphore;

use crate::plan::{Fault, ID_HEADER, Planned, SIZE_HEADER, body_for};

const BACKLOG: u32 = 65_535;

#[derive(Clone)]
pub struct Capacity {
    pub permits: Arc<Semaphore>,
    pub service: Duration,
    pub queue_limit: usize,
    pub queued: Arc<AtomicUsize>,
}

pub type Planner = Arc<dyn Fn(u64, IpAddr) -> Planned + Send + Sync>;

pub struct Origin {
    pub url: String,
    pub state: Arc<State>,
    runtime: Option<Runtime>,
}

pub struct State {
    pub track_attempts: bool,
    pub capacity: Mutex<Option<Capacity>>,
    pub faults_on: AtomicBool,
    pub accepted: AtomicU64,
    pub served: AtomicU64,
    pub open: AtomicUsize,
    pub close_every: AtomicU64,
    pub close_silently: AtomicBool,
    attempts: Mutex<HashMap<u64, u32>>,
}

impl Default for State {
    fn default() -> Self {
        Self::counting(true)
    }
}

impl State {
    fn counting(track_attempts: bool) -> Self {
        Self {
            track_attempts,
            capacity: Mutex::default(),
            faults_on: AtomicBool::new(false),
            accepted: AtomicU64::new(0),
            served: AtomicU64::new(0),
            open: AtomicUsize::new(0),
            close_every: AtomicU64::new(0),
            close_silently: AtomicBool::new(false),
            attempts: Mutex::default(),
        }
    }

    fn attempt(&self, id: u64) -> u32 {
        let mut attempts = self.attempts.lock().unwrap();
        let count = attempts.entry(id).or_default();
        *count += 1;
        *count
    }

    pub fn attempts(&self, id: u64) -> u32 {
        self.attempts.lock().unwrap().get(&id).copied().unwrap_or(0)
    }
}

struct Open(Arc<State>);

impl Drop for Open {
    fn drop(&mut self) {
        self.0.open.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Origin {
    pub fn start(threads: usize, planner: Planner) -> Self {
        Self::listen(threads, "127.0.0.1:0", planner)
    }

    pub fn listen(threads: usize, address: &str, planner: Planner) -> Self {
        let runtime = Builder::new_multi_thread()
            .worker_threads(threads)
            .enable_all()
            .build()
            .expect("origin runtime");
        let state = Arc::new(State::counting(!address.starts_with("0.0.0.0")));
        let mut bound: std::net::SocketAddr = address.parse().expect("origin address");
        for _ in 0..threads.max(1) {
            let accepting = {
                let _entered = runtime.enter();
                let socket = TcpSocket::new_v4().expect("origin socket");
                socket.set_reuseaddr(true).expect("reuse address");
                socket.set_reuseport(true).expect("reuse port");
                socket.bind(bound).expect("bind origin");
                socket.listen(BACKLOG).expect("listen")
            };
            bound = accepting.local_addr().expect("origin address");
            runtime.spawn(accept(accepting, Arc::clone(&state), Arc::clone(&planner)));
        }
        let url = format!("http://{bound}/");
        Self {
            url,
            state,
            runtime: Some(runtime),
        }
    }
}

impl Drop for Origin {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

async fn accept(listener: TcpListener, state: Arc<State>, planner: Planner) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        stream.set_nodelay(true).expect("nodelay");
        state.accepted.fetch_add(1, Ordering::Relaxed);
        state.open.fetch_add(1, Ordering::SeqCst);
        let guard = Open(Arc::clone(&state));
        tokio::spawn(serve(stream, guard, Arc::clone(&planner)));
    }
}

async fn admit(capacity: &Capacity) -> bool {
    if capacity.queued.fetch_add(1, Ordering::Relaxed) >= capacity.queue_limit {
        capacity.queued.fetch_sub(1, Ordering::Relaxed);
        return false;
    }
    let busy = Arc::clone(&capacity.permits)
        .acquire_owned()
        .await
        .expect("capacity");
    capacity.queued.fetch_sub(1, Ordering::Relaxed);
    tokio::time::sleep(capacity.service).await;
    drop(busy);
    true
}

fn head_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|at| at + 4)
}

fn field(head: &str, name: &str) -> Option<u64> {
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name)
            .then(|| value.trim().parse().ok())?
    })
}

struct Asked {
    id: u64,
    size: usize,
    closing: bool,
    control: Option<Control>,
}

#[derive(Clone, Copy)]
enum Control {
    Stats,
    FaultsOn,
    FaultsOff,
}

fn control_of(head: &str) -> Option<Control> {
    match head.split_whitespace().nth(1)? {
        "/__stats" => Some(Control::Stats),
        "/__faults/on" => Some(Control::FaultsOn),
        "/__faults/off" => Some(Control::FaultsOff),
        _ => None,
    }
}

async fn read_request(stream: &mut TcpStream, buffer: &mut Vec<u8>) -> Option<Asked> {
    let mut chunk = [0u8; 4096];
    let end = loop {
        if let Some(end) = head_end(buffer) {
            break end;
        }
        match stream.read(&mut chunk).await {
            Ok(n) if n > 0 => buffer.extend_from_slice(&chunk[..n]),
            _ => return None,
        }
    };
    let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
    let length = field(&head, "content-length").unwrap_or(0) as usize;
    while buffer.len() < end + length {
        match stream.read(&mut chunk).await {
            Ok(n) if n > 0 => buffer.extend_from_slice(&chunk[..n]),
            _ => return None,
        }
    }
    buffer.drain(..end + length);
    let control = control_of(&head);
    let id = field(&head, ID_HEADER).or(control.map(|_| 0))?;
    Some(Asked {
        id,
        size: field(&head, SIZE_HEADER).unwrap_or(0) as usize,
        closing: false,
        control,
    })
}

fn answer_control(state: &State, control: Control) -> String {
    match control {
        Control::FaultsOn => state.faults_on.store(true, Ordering::SeqCst),
        Control::FaultsOff => state.faults_on.store(false, Ordering::SeqCst),
        Control::Stats => {}
    }
    let text = format!(
        "{} {} {}",
        state.accepted.load(Ordering::Relaxed),
        state.served.load(Ordering::Relaxed),
        state.open.load(Ordering::SeqCst)
    );
    format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
        text.len()
    )
}

async fn serve(mut stream: TcpStream, guard: Open, planner: Planner) {
    let mut buffer = Vec::with_capacity(4096);
    let ip = stream.local_addr().expect("local address").ip();
    let mut answered = 0u64;
    while let Some(mut asked) = read_request(&mut stream, &mut buffer).await {
        let state = &guard.0;
        answered += 1;
        let every = state.close_every.load(Ordering::Relaxed);
        let last = every > 0 && answered >= every;
        asked.closing = last && !state.close_silently.load(Ordering::Relaxed);
        if let Some(control) = asked.control {
            write(&mut stream, answer_control(state, control).as_bytes()).await;
            return;
        }
        let capacity = state.capacity.lock().unwrap().clone();
        let shed = match capacity {
            Some(capacity) => !admit(&capacity).await,
            None => false,
        };
        let faults = state.faults_on.load(Ordering::SeqCst);
        let attempt = if state.track_attempts {
            state.attempt(asked.id)
        } else {
            1
        };
        let fault = match (shed, faults) {
            (true, _) => Fault::Busy,
            (false, true) => planner(asked.id, ip).applies(attempt),
            (false, false) => Fault::None,
        };
        if !respond(&mut stream, &asked, fault).await {
            return;
        }
        state.served.fetch_add(1, Ordering::Relaxed);
        if last {
            return;
        }
    }
}

fn ok_head(length: usize) -> String {
    format!("HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {length}\r\n\r\n")
}

async fn write(stream: &mut TcpStream, bytes: &[u8]) -> bool {
    stream.write_all(bytes).await.is_ok()
}

async fn drip(stream: &mut TcpStream, body: &str) -> bool {
    if !write(stream, ok_head(body.len()).as_bytes()).await {
        return false;
    }
    for byte in body.as_bytes() {
        tokio::time::sleep(Duration::from_millis(2)).await;
        if !write(stream, &[*byte]).await {
            return false;
        }
    }
    true
}

async fn stall(stream: &mut TcpStream) -> bool {
    let mut sink = [0u8; 256];
    while matches!(stream.read(&mut sink).await, Ok(n) if n > 0) {}
    false
}

static PADDING: [u8; 1 << 20] = [b'.'; 1 << 20];

async fn sized(stream: &mut TcpStream, id: &str, asked: &Asked) -> bool {
    let total = asked.size.max(id.len());
    let close = if asked.closing {
        "connection: close\r\n"
    } else {
        ""
    };
    let first = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\n{close}content-length: {total}\r\n\r\n{id}"
    );
    write(stream, first.as_bytes()).await && write(stream, &PADDING[..total - id.len()]).await
}

async fn respond(stream: &mut TcpStream, asked: &Asked, fault: Fault) -> bool {
    let body = body_for(asked.id);
    if fault == Fault::None {
        return sized(stream, &body, asked).await;
    }
    let whole = format!("{}{body}", ok_head(body.len()));
    match fault {
        Fault::None => unreachable!(),
        Fault::CloseBeforeResponse => false,
        Fault::CloseMidHead => {
            write(stream, b"HTTP/1.1 200 OK\r\ncontent-le").await;
            false
        }
        Fault::TruncatedLength => {
            write(stream, ok_head(body.len() + 100).as_bytes()).await;
            write(stream, body.as_bytes()).await;
            false
        }
        Fault::TruncatedChunked => {
            let chunk = format!(
                "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n",
                body.len()
            );
            write(stream, chunk.as_bytes()).await;
            false
        }
        Fault::MalformedHead => {
            write(stream, b"NOT-HTTP garbage\r\n\r\n").await;
            false
        }
        Fault::Stall => stall(stream).await,
        Fault::SlowDrip => drip(stream, &body).await,
        Fault::CloseAfterResponse => {
            write(stream, whole.as_bytes()).await;
            false
        }
        Fault::Busy => {
            write(
                stream,
                b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n",
            )
            .await
        }
        Fault::Delay(wait) => {
            tokio::time::sleep(wait).await;
            write(stream, whole.as_bytes()).await
        }
    }
}
