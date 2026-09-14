use std::convert::Infallible;
use std::env;
use std::fs;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rcgen::generate_simple_self_signed;
use rustls::ServerConfig;
use rustls::crypto::aws_lc_rs::default_provider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{CipherSuite, NamedGroup};
use tokio::net::TcpListener;
use tokio::spawn;
use tokio_rustls::TlsAcceptor;

#[tokio::main]
async fn main() {
    let mut provider = default_provider();
    if env::var("CMP_TLS").as_deref() == Ok("matched") {
        provider
            .cipher_suites
            .retain(|suite| suite.suite() == CipherSuite::TLS13_AES_128_GCM_SHA256);
        provider
            .kx_groups
            .retain(|group| group.name() == NamedGroup::X25519);
    }
    provider.install_default().expect("crypto provider");
    let (certificate, key) = if let Some(path) = env::var_os("CMP_CERT") {
        let certificate = CertificateDer::from(fs::read(path).expect("read certificate"));
        let key = PrivatePkcs8KeyDer::from(
            fs::read(env::var_os("CMP_KEY").expect("CMP_KEY")).expect("read private key"),
        );
        (certificate, PrivateKeyDer::Pkcs8(key))
    } else {
        let leaf = generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()])
            .expect("certificate");
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf.signing_key.serialize_der()));
        (leaf.cert.der().clone(), key)
    };
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key)
        .expect("TLS config");
    config.alpn_protocols = vec![b"h2".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let address = env::args().nth(1).unwrap_or_else(|| "127.0.0.1:0".into());
    let listener = TcpListener::bind(address).await.expect("bind");
    let body = match env::var_os("CMP_BODY") {
        Some(size) => Bytes::from(
            (0..size
                .to_str()
                .expect("CMP_BODY is UTF-8")
                .parse::<usize>()
                .expect("CMP_BODY is a byte count"))
                .map(|i| (i % 251) as u8)
                .collect::<Vec<_>>(),
        ),
        None => Bytes::from_static(b"ok-10byte!"),
    };
    let log = env::var("CMP_LOG_PROTO").as_deref() == Ok("1");
    println!(
        "LISTENING https://{}/",
        listener.local_addr().expect("address")
    );
    loop {
        let (socket, _) = listener.accept().await.expect("accept");
        socket.set_nodelay(true).expect("TCP_NODELAY");
        let acceptor = acceptor.clone();
        let body = body.clone();
        drop(spawn(async move {
            let stream = match acceptor.accept(socket).await {
                Ok(stream) => stream,
                Err(error) => {
                    eprintln!("TLS handshake: {error}");
                    return;
                }
            };
            if log {
                let tls = stream.get_ref().1;
                eprintln!(
                    "TLS version={:?} cipher={:?} group={:?} handshake={:?}",
                    tls.protocol_version(),
                    tls.negotiated_cipher_suite().map(|suite| suite.suite()),
                    tls.negotiated_key_exchange_group()
                        .map(|group| group.name()),
                    tls.handshake_kind()
                );
            }
            let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                let body = body.clone();
                async move {
                    if log {
                        eprintln!("PROTO {:?}", request.version());
                    }
                    Ok::<_, Infallible>(
                        Response::builder()
                            .header("content-type", "text/plain")
                            .body(Full::new(body))
                            .expect("response"),
                    )
                }
            });
            if let Err(error) = http2::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(stream), service)
                .await
            {
                eprintln!("connection: {error}");
            }
        }));
    }
}
