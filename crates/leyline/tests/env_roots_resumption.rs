#![expect(
    clippy::unwrap_used,
    reason = "test harness: unwrap doubles as the assertion"
)]
#[path = "it/tls_support/mod.rs"]
mod tls_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use leyline::{Protocol, Session, Tls};
use leyline_bssl_tokio::SslStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn reply_ok(mut stream: SslStream<TcpStream>) {
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(read) => head.extend_from_slice(&buf[..read]),
        }
    }
    drop(
        stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
            .await,
    );
}

struct EnvRestore(&'static str);

impl Drop for EnvRestore {
    fn drop(&mut self) {
        // SAFETY: this integration test binary contains only this single current_thread test.
        unsafe { std::env::remove_var(self.0) };
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_saved_tls_session_does_not_resume_after_environment_roots_change() {
    let (cert, key) = tls_support::self_signed();
    let pem = cert.to_pem().unwrap();
    let other_pem = tls_support::self_signed().0.to_pem().unwrap();
    let resumed = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&resumed);
    let port = tls_support::tls_server_per_connection(
        cert,
        key,
        Arc::new(AtomicUsize::new(0)),
        |_| tls_support::HTTP11,
        move |_, stream: SslStream<TcpStream>| {
            let counter = Arc::clone(&counter);
            async move {
                if stream.ssl().session_reused() {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
                reply_ok(stream).await;
            }
        },
    )
    .await;
    let url = format!("https://127.0.0.1:{port}/");

    let dir = std::env::temp_dir().join(format!("leyline-env-roots-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let trusted = dir.join("env-trusted.pem");
    let other = dir.join("env-other.pem");
    std::fs::write(&trusted, &pem).unwrap();
    std::fs::write(&other, &other_pem).unwrap();

    let env_trust = || Tls::new().system_roots(false).env_roots(true);
    let _restore = EnvRestore("SSL_CERT_FILE");
    // SAFETY: this integration test binary contains only this single current_thread test.
    unsafe { std::env::set_var("SSL_CERT_FILE", &trusted) };
    let before = Session::builder()
        .protocol(Protocol::Http1)
        .tls(env_trust())
        .build()
        .unwrap();
    let first = before.get(&url).send().await.unwrap();
    assert_eq!(first.text().await.unwrap(), "ok");
    let state = leyline::internals::connection_state(&before);

    // SAFETY: this integration test binary contains only this single current_thread test.
    unsafe { std::env::set_var("SSL_CERT_FILE", &other) };
    let after = Session::builder()
        .protocol(Protocol::Http1)
        .tls(env_trust())
        .build()
        .unwrap();
    drop(_restore);
    leyline::internals::restore(&after, &state);
    let refused = after.get(&url).send().await.unwrap_err();
    assert!(
        std::error::Error::source(&refused)
            .and_then(|s| s.downcast_ref::<leyline::internals::TlsError>())
            .is_some(),
        "{refused:?}"
    );
    assert_eq!(resumed.load(Ordering::SeqCst), 0);
    drop(std::fs::remove_dir_all(&dir));
}
