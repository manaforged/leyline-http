#![cfg(feature = "http3")]
#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
#[path = "tls_support/mod.rs"]
mod tls_support;

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::StreamExt;
use leyline::{
    Browser, CompressionConfig, Kind, ProtocolPolicy, RetryPolicy, Session, TimeoutConfig,
    TlsTrustConfig,
};
use leyline_bssl::pkey::{PKey, Private};
use leyline_bssl::ssl::{SslContextBuilder, SslMethod};
use leyline_bssl::x509::X509;
use tokio::net::UdpSocket;

const CAP: usize = 1024;
const SMALL: usize = 16;
const LARGE: usize = 4096;
const DATAGRAM: usize = 1350;
const IDLE: Duration = Duration::from_millis(50);

fn server_config(cert: &X509, key: &PKey<Private>) -> leyline_quiche::Config {
    let mut tls = SslContextBuilder::new(SslMethod::tls()).unwrap();
    tls.set_certificate(cert).unwrap();
    tls.set_private_key(key).unwrap();
    let mut config =
        leyline_quiche::Config::with_boring_ssl_ctx_builder(leyline_quiche::PROTOCOL_VERSION, tls)
            .unwrap();
    config
        .set_application_protos(leyline_quiche::h3::APPLICATION_PROTOCOL)
        .unwrap();
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

struct Peer {
    quic: leyline_quiche::Connection,
    h3: Option<leyline_quiche::h3::Connection>,
    answered: usize,
}

impl Peer {
    fn respond(&mut self, sizes: &[usize]) {
        let (Some(h3), quic) = (self.h3.as_mut(), &mut self.quic) else {
            return;
        };
        while let Ok((stream, event)) = h3.poll(quic) {
            if !matches!(event, leyline_quiche::h3::Event::Headers { .. }) {
                continue;
            }
            let length = sizes[self.answered.min(sizes.len() - 1)];
            let status = [leyline_quiche::h3::Header::new(b":status", b"200")];
            h3.send_response(quic, stream, &status, false).unwrap();
            h3.send_body(quic, stream, &vec![b'x'; length], true)
                .unwrap();
            self.answered += 1;
        }
    }
}

async fn serve(socket: UdpSocket, mut config: leyline_quiche::Config, sizes: Vec<usize>) {
    let local = socket.local_addr().unwrap();
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
                    let quic =
                        leyline_quiche::accept(&scid, None, local, from, &mut config).unwrap();
                    peer = Some((
                        from,
                        Peer {
                            quic,
                            h3: None,
                            answered: 0,
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
            let h3_config = leyline_quiche::h3::Config::new().unwrap();
            state.h3 = Some(
                leyline_quiche::h3::Connection::with_transport(&mut state.quic, &h3_config)
                    .unwrap(),
            );
        }
        state.respond(&sizes);
        while let Ok((len, _)) = state.quic.send(&mut outbound) {
            let _ = socket.send_to(&outbound[..len], *address).await;
        }
        if state.quic.is_closed() {
            return;
        }
    }
}

async fn h3_server(sizes: Vec<usize>) -> (u16, Vec<u8>) {
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let config = server_config(&cert, &key);
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    tokio::spawn(serve(socket, config, sizes));
    (port, der)
}

fn capped(der: Vec<u8>) -> Session {
    Session::builder()
        .browser(Browser::Chrome154)
        .protocol(ProtocolPolicy::Http3)
        .compression(CompressionConfig::new().max_body_size(CAP))
        .tls_trust(
            TlsTrustConfig::new()
                .env_roots(false)
                .system_roots(false)
                .add_ca_der(der),
        )
        .retry(RetryPolicy::none())
        .timeout(TimeoutConfig::new().total(Duration::from_secs(10)))
        .build()
        .unwrap()
}

#[tokio::test]
async fn an_h3_body_over_the_cap_is_a_body_error() {
    let (port, der) = h3_server(vec![SMALL, LARGE]).await;
    let session = capped(der);
    let url = format!("https://127.0.0.1:{port}/");
    let first = session.get(&url).await.unwrap().bytes().await.unwrap();
    assert_eq!(first.len(), SMALL);
    let second = async { session.get(&url).await?.bytes().await }.await;
    let err = second.unwrap_err();
    assert_eq!(err.kind(), Kind::Body, "{err:?}");
}

#[tokio::test]
async fn a_streamed_h3_body_is_not_capped() {
    let (port, der) = h3_server(vec![LARGE]).await;
    let session = capped(der);
    let url = format!("https://127.0.0.1:{port}/");
    let mut body = session
        .get(&url)
        .stream()
        .await
        .unwrap()
        .into_stream()
        .unwrap();
    let mut read = 0;
    while let Some(chunk) = body.next().await {
        read += chunk.unwrap().len();
    }
    assert_eq!(read, LARGE);
}
