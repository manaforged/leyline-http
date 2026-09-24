use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper::{Request, Response, Version};
use hyper_util::rt::{TokioExecutor, TokioIo};
use leyline::Session;
use rcgen::generate_simple_self_signed;
use rustls::ServerConfig;
use rustls::crypto::aws_lc_rs::default_provider;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::net::TcpListener;
use tokio::time::{sleep, timeout};
use tokio_rustls::TlsAcceptor;

const SIZE: usize = 4 * 1024 * 1024;

async fn receive(delay: Duration, abandon: bool) {
    let _ = default_provider().install_default();
    let leaf = generate_simple_self_signed(vec!["127.0.0.1".into()]).expect("certificate");
    let certificate = leaf.cert.der().clone();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf.signing_key.serialize_der()));
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate.clone()], key)
        .expect("TLS config");
    config.alpn_protocols = vec![b"h2".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let url = format!("https://{}/", listener.local_addr().expect("address"));
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("accept");
        socket.set_nodelay(true).expect("TCP_NODELAY");
        let stream = acceptor.accept(socket).await.expect("TLS");
        let body = Bytes::from((0..SIZE).map(|i| (i % 251) as u8).collect::<Vec<_>>());
        let service = service_fn(move |request: Request<hyper::body::Incoming>| {
            let body = body.clone();
            async move {
                assert_eq!(request.version(), Version::HTTP_2);
                Ok::<_, Infallible>(Response::new(Full::new(body)))
            }
        });
        http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(stream), service)
            .await
    });
    let response = timeout(
        Duration::from_secs(3),
        Session::builder()
            .protocol(leyline::ProtocolPolicy::Http2)
            .tls_trust(
                leyline::TlsTrustConfig::new()
                    .without_system_roots()
                    .without_env_roots()
                    .add_ca_der(certificate.to_vec()),
            )
            .build()
            .expect("session")
            .get(&url)
            .stream(),
    )
    .await
    .expect("response deadline")
    .expect("response headers");
    assert_eq!(response.status().as_u16(), 200);
    sleep(delay).await;
    if abandon {
        drop(response);
    } else {
        let body = timeout(Duration::from_secs(3), response.bytes())
            .await
            .expect("body deadline")
            .expect("complete body after session drop");
        assert_eq!(body.len(), SIZE);
        assert!(
            body.iter()
                .enumerate()
                .all(|(i, byte)| *byte == (i % 251) as u8)
        );
    }
    let result = timeout(Duration::from_secs(3), server)
        .await
        .expect("connection released")
        .expect("origin task");
    if !abandon {
        result.expect("origin completed");
    }
}

#[tokio::test]
async fn temporary_session_completes_large_stream() {
    receive(Duration::ZERO, false).await;
}

#[tokio::test]
async fn slow_body_outlives_temporary_session() {
    receive(Duration::from_millis(750), false).await;
}

#[tokio::test]
async fn abandoned_body_releases_temporary_session_connection() {
    receive(Duration::from_millis(20), true).await;
}
