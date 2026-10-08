#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use crate::tls_support;

use leyline::{Browser, Platform, Session, TlsMinVersion, TlsTrustConfig};
use leyline_bssl::pkey::PKey;
use leyline_bssl::ssl::{Ssl, SslContextBuilder, SslMethod, SslVersion};
use leyline_bssl::x509::X509;
use tls_support::self_signed;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn tls11_server(cert: X509, key: PKey<leyline_bssl::pkey::Private>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut context = SslContextBuilder::new(SslMethod::tls()).unwrap();
    context.set_certificate(&cert).unwrap();
    context.set_private_key(&key).unwrap();
    context
        .set_min_proto_version(Some(SslVersion::TLS1))
        .unwrap();
    context
        .set_max_proto_version(Some(SslVersion::TLS1_1))
        .unwrap();
    let context = context.build();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let ssl = Ssl::new(&context).unwrap();
            if let Ok(mut stream) = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                .accept()
                .await
            {
                let mut buf = [0u8; 4096];
                drop(stream.read(&mut buf).await);
                drop(
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                        )
                        .await,
                );
            }
        }
    });
    port
}

fn safari(trust: TlsTrustConfig) -> Session {
    Session::builder()
        .browser(Browser::Safari18)
        .platform(Platform::MacOS)
        .tls_trust(trust)
        .build()
        .unwrap()
}

#[tokio::test]
async fn a_tls_floor_refuses_a_server_below_it() {
    let (cert, key) = self_signed();
    let der = cert.to_der().unwrap();
    let port = tls11_server(cert, key).await;
    let url = format!("https://127.0.0.1:{port}/");
    let trust = TlsTrustConfig::new()
        .env_roots(false)
        .system_roots(false)
        .add_ca_der(der);
    let open = safari(trust.clone()).get(&url).await;
    assert!(open.is_ok(), "{open:?}");
    let floored = safari(trust.min_tls_version(TlsMinVersion::Tls12))
        .get(&url)
        .await;
    assert!(floored.is_err(), "{floored:?}");
}
