#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

fn server_config(cert: &X509, key: &PKey<Private>) -> leyline_quiche::Config {
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
    config.set_initial_max_streams_bidi(100);
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
}

#[derive(Default)]
struct Seen {
    stopped: Vec<u64>,
    peer_close: Option<u64>,
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

    pub fn peer_close(&self) -> Option<u64> {
        self.seen.lock().expect("seen").peer_close
    }
}

struct Peer {
    quic: leyline_quiche::Connection,
    h3: Option<leyline_quiche::h3::Connection>,
    answered: usize,
    held: Vec<u64>,
}

fn status(code: &[u8]) -> [leyline_quiche::h3::Header; 1] {
    [leyline_quiche::h3::Header::new(b":status", code)]
}

impl Peer {
    fn respond(&mut self, replies: &[Reply]) {
        let (Some(h3), quic) = (self.h3.as_mut(), &mut self.quic) else {
            return;
        };
        while let Ok((stream, event)) = h3.poll(quic) {
            if !matches!(event, leyline_quiche::h3::Event::Headers { .. }) {
                continue;
            }
            match replies[self.answered.min(replies.len() - 1)] {
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
                Reply::Reset(code) => {
                    let _ = quic.stream_shutdown(stream, leyline_quiche::Shutdown::Write, code);
                    let _ = quic.stream_shutdown(stream, leyline_quiche::Shutdown::Read, code);
                }
            }
            self.answered += 1;
        }
    }
}

impl Peer {
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
    let mut peer: Option<(SocketAddr, Peer)> = None;
    loop {
        let wait = peer
            .as_ref()
            .and_then(|(_, peer)| peer.quic.timeout())
            .unwrap_or(IDLE);
        match tokio::time::timeout(wait, socket.recv_from(&mut inbound)).await {
            Ok(Ok((len, from))) => {
                if peer.is_none() {
                    let scid = leyline_quiche::ConnectionId::from_vec(vec![7; 16]);
                    let quic = leyline_quiche::accept(&scid, None, local, from, &mut config)
                        .expect("accept");
                    peer = Some((
                        from,
                        Peer {
                            quic,
                            h3: None,
                            answered: 0,
                            held: Vec::new(),
                        },
                    ));
                }
                if let Some((_, peer)) = peer.as_mut() {
                    let info = leyline_quiche::RecvInfo { from, to: local };
                    let _ = peer.quic.recv(&mut inbound[..len], info);
                }
            }
            Ok(Err(_)) => return,
            Err(_) => {
                if let Some((_, peer)) = peer.as_mut() {
                    peer.quic.on_timeout();
                }
            }
        }
        let Some((address, state)) = peer.as_mut() else {
            continue;
        };
        if state.h3.is_none() && state.quic.is_established() {
            let h3_config = leyline_quiche::h3::Config::new().expect("HTTP/3 config");
            state.h3 = Some(
                leyline_quiche::h3::Connection::with_transport(&mut state.quic, &h3_config)
                    .expect("HTTP/3 connection"),
            );
        }
        state.respond(&replies);
        state.watch(&seen);
        while let Ok((len, _)) = state.quic.send(&mut outbound) {
            let _ = socket.send_to(&outbound[..len], *address).await;
        }
        if state.quic.is_closed() {
            return;
        }
    }
}

pub async fn h3_server(replies: Vec<Reply>) -> H3Server {
    let (cert, key) = crate::tls_support::self_signed();
    let der = cert.to_der().expect("certificate DER");
    let config = server_config(&cert, &key);
    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
    let port = socket.local_addr().expect("local address").port();
    let seen = Arc::new(Mutex::new(Seen::default()));
    tokio::spawn(serve(socket, config, replies, Arc::clone(&seen)));
    H3Server { port, der, seen }
}
