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
    location
        .set(format!("http://127.0.0.1:{}/result", closed_port()))
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
    let err = unsent_retries()
        .post(format!("http://127.0.0.1:{}/create-job", closed_port()))
        .body("job=1")
        .await
        .unwrap_err();

    assert!(err.is_connect(), "{err}");
    assert_eq!(err.attempts(), 4);
}
