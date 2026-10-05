#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use leyline::testing::{TestResponse, TestServer};
use leyline::{RetryPolicy, Session};

fn closed_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn unsent_retries() -> Session {
    Session::builder()
        .retry(
            RetryPolicy::transient()
                .initial_backoff(Duration::from_millis(1))
                .jitter(false)
                .retry_unsent(true),
        )
        .build()
        .unwrap()
}

#[tokio::test]
async fn a_post_answered_with_a_redirect_is_not_replayed_when_the_next_leg_fails() {
    let location = Arc::new(OnceLock::new());
    let target = Arc::clone(&location);
    let server = TestServer::http(move |_| {
        TestResponse::new(303).header("location", target.get().cloned().unwrap_or_default())
    })
    .await
    .unwrap();
    let port = closed_port();
    location
        .set(format!("http://127.0.0.1:{port}/result"))
        .unwrap();

    let err = unsent_retries()
        .post(server.url("/create-job"))
        .body("job=1")
        .await
        .unwrap_err();

    assert!(err.is_connect(), "{err}");
    assert_eq!(server.requests().await.len(), 1);
}

#[tokio::test]
async fn a_post_whose_first_leg_never_connects_is_still_retried() {
    let port = closed_port();
    let err = unsent_retries()
        .post(format!("http://127.0.0.1:{port}/create-job"))
        .body("job=1")
        .await
        .unwrap_err();

    assert!(err.is_connect(), "{err}");
    assert_eq!(err.attempts(), 4);
}

#[tokio::test]
async fn a_digest_challenge_does_not_block_retrying_an_unsent_post() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let challenger = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            head.push(byte[0]);
        }
        stream
            .write_all(
                b"HTTP/1.1 401 Unauthorized\r\n\
                  WWW-Authenticate: Digest realm=\"r\", nonce=\"n1\", qop=\"auth\", algorithm=MD5\r\n\
                  Connection: close\r\nContent-Length: 0\r\n\r\n",
            )
            .await
            .unwrap();
    });

    let err = unsent_retries()
        .post(format!("http://127.0.0.1:{port}/create-job"))
        .digest_auth(leyline::DigestAuth::new("user", "pass"))
        .body("job=1")
        .await
        .unwrap_err();
    challenger.await.unwrap();

    assert!(err.is_connect(), "{err}");
    assert_eq!(err.attempts(), 4);
}
