use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::{Connect, Done, Head, Sent, Trace};
use crate::Session;

#[derive(Default)]
struct Recorder {
    lines: Mutex<Vec<String>>,
    ids: Mutex<Vec<u64>>,
}

impl Recorder {
    fn lines(&self) -> Vec<String> {
        lock(&self.lines).clone()
    }

    fn ids(&self) -> Vec<u64> {
        lock(&self.ids).clone()
    }

    fn push(&self, id: u64, line: String) {
        lock(&self.lines).push(line);
        lock(&self.ids).push(id);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Trace for Recorder {
    fn connect(&self, ev: &Connect<'_>) {
        self.push(ev.id, format!("connect reused={}", ev.reused));
    }

    fn sent(&self, ev: &Sent<'_>) {
        self.push(ev.id, "sent".to_string());
    }

    fn head(&self, ev: &Head<'_>) {
        self.push(ev.id, format!("head status={}", ev.status));
    }

    fn done(&self, ev: &Done<'_>) {
        self.push(ev.id, format!("done ok={}", ev.outcome.is_ok()));
    }
}

async fn serve(count: usize) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        for _ in 0..count {
            let mut buf = [0u8; 4096];
            let read = sock.read(&mut buf).await.unwrap();
            if read == 0 {
                return;
            }
            sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nhi")
                .await
                .unwrap();
        }
    });
    addr
}

fn session(hook: &Arc<Recorder>) -> Session {
    Session::builder().trace(Arc::clone(hook)).build().unwrap()
}

#[tokio::test]
async fn order_is_connect_sent_head_done() {
    let addr = serve(1).await;
    let hook = Arc::new(Recorder::default());
    let session = session(&hook);

    let resp = session
        .get(&format!("http://{addr}/one"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);

    let lines = hook.lines();
    assert_eq!(
        lines,
        vec![
            "connect reused=false".to_string(),
            "sent".to_string(),
            "head status=200".to_string(),
            "done ok=true".to_string(),
        ],
        "unexpected event order: {lines:?}"
    );
    let ids = hook.ids();
    assert!(
        ids.windows(2).all(|w| w[0] == w[1]),
        "one attempt shares one id: {ids:?}"
    );
}

#[tokio::test]
async fn a_pooled_second_request_reports_reuse() {
    let addr = serve(2).await;
    let hook = Arc::new(Recorder::default());
    let session = session(&hook);

    drop(
        session
            .get(&format!("http://{addr}/one"))
            .send()
            .await
            .unwrap(),
    );
    let first = hook.lines().len();
    drop(
        session
            .get(&format!("http://{addr}/two"))
            .send()
            .await
            .unwrap(),
    );

    let second = hook.lines().split_off(first);
    assert_eq!(
        second,
        vec![
            "connect reused=true".to_string(),
            "sent".to_string(),
            "head status=200".to_string(),
            "done ok=true".to_string(),
        ],
        "the pooled attempt should report reuse: {second:?}"
    );

    let ids = hook.ids();
    assert_ne!(ids[0], ids[first], "each attempt gets its own id: {ids:?}");
}

#[tokio::test]
async fn done_carries_the_error() {
    let hook = Arc::new(Recorder::default());
    let session = session(&hook);

    drop(
        session
            .get("http://127.0.0.1:1/refused")
            .send()
            .await
            .unwrap_err(),
    );

    let lines = hook.lines();
    assert_eq!(lines.last().map(String::as_str), Some("done ok=false"));
}

#[tokio::test]
async fn response_timing_reports_a_fresh_dial() {
    let addr = serve(1).await;
    let session = Session::builder().build().unwrap();
    let resp = session
        .get(&format!("http://{addr}/one"))
        .send()
        .await
        .unwrap();

    let timing = resp.timing();
    assert!(!timing.reused, "a fresh dial is not a reuse");
    assert!(timing.connect_ms.is_some(), "a fresh dial records connect");
}

#[tokio::test]
async fn no_listener_fires_nothing() {
    let addr = serve(1).await;
    let session = Session::builder().build().unwrap();
    let resp = session
        .get(&format!("http://{addr}/one"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert!(!super::on(), "no scope outside a traced request");
}
