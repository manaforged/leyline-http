#![allow(
    dead_code,
    reason = "shared by several test binaries; each binary uses a subset"
)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use leyline::{
    Browser, ProtocolPolicy, RetryPolicy, Session, SessionBuilder, TimeoutConfig, TlsTrustConfig,
};
use leyline_bssl::pkey::{PKey, Private};
use leyline_bssl::ssl::{SslContextBuilder, SslMethod};
use leyline_bssl::x509::X509;
use tokio::net::UdpSocket;

const DATAGRAM: usize = 1350;
const IDLE: Duration = Duration::from_millis(50);
const WATCH: Duration = Duration::from_secs(5);
const GOAWAY_LAG: Duration = Duration::from_millis(200);
const NEXT_REQUEST: u64 = 4;

#[derive(Clone, Copy)]
pub struct Limits {
    pub streams: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { streams: 100 }
    }
}

fn server_config(cert: &X509, key: &PKey<Private>, limits: Limits) -> leyline_quiche::Config {
    let mut tls = SslContextBuilder::new(SslMethod::tls()).expect("TLS context");
    tls.set_certificate(cert).expect("certificate");
    tls.set_private_key(key).expect("private key");
    let mut config =
        leyline_quiche::Config::with_boring_ssl_ctx_builder(leyline_quiche::PROTOCOL_VERSION, tls)
            .expect("QUIC config");
    config
        .set_application_protos(leyline_quiche::h3::APPLICATION_PROTOCOL)
        .expect("ALPN");
    config.set_max_idle_timeout(5_000);
    config.set_max_recv_udp_payload_size(DATAGRAM);
    config.set_max_send_udp_payload_size(DATAGRAM);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local(1_000_000);
    config.set_initial_max_stream_data_bidi_remote(1_000_000);
    config.set_initial_max_stream_data_uni(1_000_000);
    config.set_initial_max_streams_bidi(limits.streams);
    config.set_initial_max_streams_uni(100);
    config.set_disable_active_migration(true);
    config
}

#[derive(Clone, Copy)]
pub enum Reply {
    Body(usize),
    Hold(usize),
    Status(&'static [u8]),
    Reset(u64),
    ResetResponse(u64),
    GoawayThenReset(u64),
    Truncate(usize, u64),
    Encoded(usize),
    Flood(usize),
}

#[derive(Clone, Copy)]
enum Halves {
    Both,
    Response,
}

#[derive(Default)]
struct Seen {
    stopped: Vec<u64>,
    peer_close: Option<u64>,
    connections: usize,
}

pub struct H3Server {
    pub port: u16,
    der: Vec<u8>,
    seen: Arc<Mutex<Seen>>,
}

impl H3Server {
    pub fn url(&self) -> String {
        format!("https://127.0.0.1:{}/", self.port)
    }

    pub fn session(&self) -> SessionBuilder {
        Session::builder()
            .browser(Browser::Chrome154)
            .protocol(ProtocolPolicy::Http3)
            .tls_trust(
                TlsTrustConfig::new()
                    .env_roots(false)
                    .system_roots(false)
                    .add_ca_der(self.der.clone()),
            )
            .retry(RetryPolicy::none())
            .timeout(TimeoutConfig::new().total(Duration::from_secs(10)))
    }

    pub async fn stop_code(&self) -> Option<u64> {
        let deadline = tokio::time::Instant::now() + WATCH;
        while tokio::time::Instant::now() < deadline {
            if let Some(&code) = self.seen.lock().expect("seen").stopped.first() {
                return Some(code);
            }
            tokio::time::sleep(IDLE).await;
        }
        None
    }

    pub fn connections(&self) -> usize {
        self.seen.lock().expect("seen").connections
    }

    pub fn peer_close(&self) -> Option<u64> {
        self.seen.lock().expect("seen").peer_close
    }
}

struct Deferred {
    stream: u64,
    code: u64,
    halves: Halves,
    due: Instant,
}

struct Peer {
    quic: leyline_quiche::Connection,
    h3: Option<leyline_quiche::h3::Connection>,
    held: Vec<u64>,
    deferred: Vec<Deferred>,
    flooding: Vec<(u64, usize)>,
}

fn status(code: &[u8]) -> [leyline_quiche::h3::Header; 1] {
    [leyline_quiche::h3::Header::new(b":status", code)]
}

fn reset(quic: &mut leyline_quiche::Connection, stream: u64, code: u64, halves: Halves) {
    if let Err(e) = quic.stream_shutdown(stream, leyline_quiche::Shutdown::Write, code) {
        eprintln!("h3 test peer: {e:?}");
    }
    if let Halves::Both = halves
        && let Err(e) = quic.stream_shutdown(stream, leyline_quiche::Shutdown::Read, code)
    {
        eprintln!("h3 test peer: {e:?}");
    }
}

impl Peer {
    fn respond(&mut self, replies: &[Reply], answered: &mut usize) {
        let (Some(h3), quic) = (self.h3.as_mut(), &mut self.quic) else {
            return;
        };
        let mut sink = vec![0u8; DATAGRAM];
        while let Ok((stream, event)) = h3.poll(quic) {
            if matches!(event, leyline_quiche::h3::Event::Data) {
                while matches!(h3.recv_body(quic, stream, &mut sink), Ok(read) if read > 0) {}
            }
            if !matches!(event, leyline_quiche::h3::Event::Headers { .. }) {
                continue;
            }
            match replies[(*answered).min(replies.len() - 1)] {
                Reply::Body(length) => {
                    h3.send_response(quic, stream, &status(b"200"), false)
                        .expect("response head");
                    h3.send_body(quic, stream, &vec![b'x'; length], true)
                        .expect("response body");
                }
                Reply::Hold(length) => {
                    h3.send_response(quic, stream, &status(b"200"), false)
                        .expect("response head");
                    h3.send_body(quic, stream, &vec![b'x'; length], false)
                        .expect("response body");
                    self.held.push(stream);
                }
                Reply::Status(code) => {
                    h3.send_response(quic, stream, &status(code), false)
                        .expect("response head");
                    self.held.push(stream);
                }
                Reply::Reset(code) => reset(quic, stream, code, Halves::Both),
                Reply::ResetResponse(code) => reset(quic, stream, code, Halves::Response),
                Reply::GoawayThenReset(code) => {
                    h3.send_goaway(quic, stream + NEXT_REQUEST).expect("GOAWAY");
                    self.deferred.push(Deferred {
                        stream,
                        code,
                        halves: Halves::Both,
                        due: Instant::now() + GOAWAY_LAG,
                    });
                }
                Reply::Encoded(length) => {
                    let declared = (length * 2).to_string();
                    let head = [
                        leyline_quiche::h3::Header::new(b":status", b"200"),
                        leyline_quiche::h3::Header::new(b"content-encoding", b"gzip"),
                        leyline_quiche::h3::Header::new(b"content-length", declared.as_bytes()),
                    ];
                    h3.send_response(quic, stream, &head, false)
                        .expect("response head");
                    let mut body = vec![0x1f, 0x8b, 0x07, 0x00];
                    body.resize(length, b'x');
                    h3.send_body(quic, stream, &body, false)
                        .expect("response body");
                    self.held.push(stream);
                }
                Reply::Flood(length) => {
                    h3.send_response(quic, stream, &status(b"200"), false)
                        .expect("response head");
                    self.flooding.push((stream, length));
                    self.held.push(stream);
                }
                Reply::Truncate(length, code) => {
                    h3.send_response(quic, stream, &status(b"200"), false)
                        .expect("response head");
                    if let Err(e) = h3.send_body(quic, stream, &vec![b'x'; length], false) {
                        eprintln!("h3 test peer: {e:?}");
                    }
                    self.deferred.push(Deferred {
                        stream,
                        code,
                        halves: Halves::Response,
                        due: Instant::now(),
                    });
                }
            }
            *answered += 1;
        }
    }

    fn flood(&mut self) {
        let (Some(h3), quic) = (self.h3.as_mut(), &mut self.quic) else {
            return;
        };
        let chunk = vec![b'x'; 16 * 1024];
        self.flooding.retain_mut(|(stream, remaining)| {
            while *remaining > 0 {
                let take = (*remaining).min(chunk.len());
                match h3.send_body(quic, *stream, &chunk[..take], false) {
                    Ok(written) if written > 0 => *remaining -= written,
                    _ => return true,
                }
            }
            false
        });
    }

    fn reset_due(&mut self) {
        let now = Instant::now();
        let quic = &mut self.quic;
        self.deferred.retain(|deferred| {
            if deferred.due > now {
                return true;
            }
            reset(quic, deferred.stream, deferred.code, deferred.halves);
            false
        });
    }

    fn watch(&mut self, seen: &Mutex<Seen>) {
        let mut seen = seen.lock().expect("seen");
        let quic = &mut self.quic;
        self.held
            .retain(|&stream| match quic.stream_capacity(stream) {
                Err(leyline_quiche::Error::StreamStopped(code)) => {
                    seen.stopped.push(code);
                    false
                }
                Err(_) => false,
                Ok(_) => true,
            });
        if seen.peer_close.is_none() {
            seen.peer_close = quic.peer_error().map(|error| error.error_code);
        }
    }

    fn advance(&mut self, replies: &[Reply], answered: &mut usize, seen: &Mutex<Seen>) {
        if self.h3.is_none() && self.quic.is_established() {
            let h3_config = leyline_quiche::h3::Config::new().expect("HTTP/3 config");
            self.h3 = Some(
                leyline_quiche::h3::Connection::with_transport(&mut self.quic, &h3_config)
                    .expect("HTTP/3 connection"),
            );
        }
        self.reset_due();
        self.respond(replies, answered);
        self.flood();
        self.watch(seen);
    }
}

fn accept(
    peers: &mut HashMap<SocketAddr, Peer>,
    datagram: &mut [u8],
    from: SocketAddr,
    local: SocketAddr,
    config: &mut leyline_quiche::Config,
) {
    let Ok(header) = leyline_quiche::Header::from_slice(datagram, leyline_quiche::MAX_CONN_ID_LEN)
    else {
        return;
    };
    if header.ty != leyline_quiche::Type::Initial {
        return;
    }
    let scid = leyline_quiche::ConnectionId::from_vec(vec![7; 16]);
    let quic = leyline_quiche::accept(&scid, None, local, from, config).expect("accept");
    let peer = Peer {
        quic,
        h3: None,
        held: Vec::new(),
        flooding: Vec::new(),
        deferred: Vec::new(),
    };
    peers.insert(from, peer);
}

async fn serve(
    socket: UdpSocket,
    mut config: leyline_quiche::Config,
    replies: Vec<Reply>,
    seen: Arc<Mutex<Seen>>,
) {
    let local = socket.local_addr().expect("local address");
    let mut inbound = vec![0u8; 65_535];
    let mut outbound = vec![0u8; DATAGRAM];
    let mut peers: HashMap<SocketAddr, Peer> = HashMap::new();
    let mut answered = 0;
    loop {
        let wait = peers
            .values()
            .filter_map(|peer| peer.quic.timeout())
            .min()
            .map_or(IDLE, |timeout| timeout.min(IDLE));
        match tokio::time::timeout(wait, socket.recv_from(&mut inbound)).await {
            Ok(Ok((len, from))) => {
                if !peers.contains_key(&from) {
                    accept(&mut peers, &mut inbound[..len], from, local, &mut config);
                    if peers.contains_key(&from) {
                        seen.lock().expect("seen").connections += 1;
                    }
                }
                if let Some(peer) = peers.get_mut(&from) {
                    let info = leyline_quiche::RecvInfo { from, to: local };
                    if let Err(e) = peer.quic.recv(&mut inbound[..len], info) {
                        eprintln!("h3 test peer: {e:?}");
                    }
                }
            }
            Ok(Err(_)) => return,
            Err(_) => peers.values_mut().for_each(|peer| peer.quic.on_timeout()),
        }
        for (address, peer) in &mut peers {
            peer.advance(&replies, &mut answered, &seen);
            while let Ok((len, _)) = peer.quic.send(&mut outbound) {
                drop(socket.send_to(&outbound[..len], *address).await);
            }
        }
        peers.retain(|_, peer| !peer.quic.is_closed());
    }
}

pub async fn h3_server(replies: Vec<Reply>, limits: Limits) -> H3Server {
    let (cert, key) = crate::tls_support::self_signed();
    let der = cert.to_der().expect("certificate DER");
    let config = server_config(&cert, &key, limits);
    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
    let port = socket.local_addr().expect("local address").port();
    let seen = Arc::new(Mutex::new(Seen::default()));
    tokio::spawn(serve(socket, config, replies, Arc::clone(&seen)));
    H3Server { port, der, seen }
}
