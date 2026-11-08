use crate::core::retry::Retry;
use crate::core::retry::RetryPlan;
use crate::error::{Error, Kind};

fn io(kind: std::io::ErrorKind) -> crate::error::Result<crate::core::response::Response> {
    Err(Error::new(Kind::Io).with_source(std::io::Error::new(kind, "x")))
}

#[test]
fn transient_retries_connection_loss_only() {
    let policy = Retry::new(3);
    assert!(matches!(
        policy.plan(&io(std::io::ErrorKind::ConnectionReset), 0),
        RetryPlan::Backoff(_)
    ));
    assert!(matches!(
        policy.plan(&io(std::io::ErrorKind::PermissionDenied), 0),
        RetryPlan::Stop
    ));
    assert!(matches!(
        policy.plan(&Err(Error::new(Kind::Timeout)), 0),
        RetryPlan::Backoff(_)
    ));
}

#[test]
fn no_policy_never_retries() {
    let policy = Retry::new(0);
    assert!(matches!(
        policy.plan(&io(std::io::ErrorKind::ConnectionReset), 0),
        RetryPlan::Stop
    ));
}

#[tokio::test]
async fn a_retried_attempt_sends_the_same_request_again() {
    use crate::testing::{Response, Server};
    use std::sync::{Arc, Mutex};
    let seen: Arc<Mutex<Vec<crate::testing::Request>>> = Arc::default();
    let log = Arc::clone(&seen);
    let server = Server::http(move |request| {
        let mut log = log.lock().expect("log");
        log.push(request.clone());
        if log.len() == 1 {
            Response::new(503)
        } else {
            Response::new(200).body("ok")
        }
    })
    .await
    .expect("server starts");
    let session = crate::Session::builder()
        .browser(crate::Browser::Chrome152)
        .retry(1)
        .build()
        .expect("session");

    let response = session
        .put(server.url("/item?id=7"))
        .header("x-probe", "1")
        .body("payload")
        .send()
        .await
        .expect("retried response");

    assert_eq!(response.status(), 200);
    let seen = seen.lock().expect("log");
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].method, seen[1].method);
    assert_eq!(seen[0].target, seen[1].target);
    assert_eq!(seen[0].headers, seen[1].headers);
    assert_eq!(seen[0].body, seen[1].body);
    assert_eq!(seen[1].body, b"payload");
}

#[tokio::test]
async fn request_builder_awaits_response_text_and_bytes_directly() {
    use crate::testing::{Response, Server};
    let server = Server::http(|request| match request.target.as_str() {
        "/text" => Response::new(200).body("hello leyline"),
        "/bytes" => Response::new(200).body(&b"\x01\x02\x03"[..]),
        _ => Response::new(204),
    })
    .await
    .expect("server starts");
    let session = crate::Session::new();

    let response = session
        .get(server.url("/head"))
        .await
        .expect("direct await");
    assert_eq!(response.status(), 204);

    let text = session
        .get(server.url("/text"))
        .text()
        .await
        .expect("direct text");
    assert_eq!(text, "hello leyline");

    let bytes = session
        .get(server.url("/bytes"))
        .bytes()
        .await
        .expect("direct bytes");
    assert_eq!(&bytes[..], b"\x01\x02\x03");
}
