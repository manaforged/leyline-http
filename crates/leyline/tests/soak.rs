#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use leyline::testing::{RecordedRequest, TestResponse, TestServer};
use leyline::{Body, HostLimits, Session, TimeoutConfig};

const CYCLES: usize = 12;
const REQUESTS: usize = 60;
const SLACK_TASKS: usize = 4;
const SLACK_FDS: usize = 8;

fn reply(request: &RecordedRequest) -> TestResponse {
    match request.target.as_str() {
        "/ok" => TestResponse::new(200).body(vec![b'o'; 2048]),
        "/slow" => TestResponse::new(200).chunks(["a", "b", "c", "d"], Duration::from_millis(150)),
        "/error" => TestResponse::new(503).body(vec![b'e'; 512]),
        "/stall" => TestResponse::new(200).delay(Duration::from_millis(300)),
        _ => TestResponse::new(200),
    }
}

fn open_fds() -> usize {
    let dir = if cfg!(target_os = "linux") {
        "/proc/self/fd"
    } else {
        "/dev/fd"
    };
    std::fs::read_dir(dir).map(Iterator::count).unwrap_or(0)
}

async fn settle() -> (usize, usize) {
    tokio::time::sleep(Duration::from_millis(500)).await;
    let tasks = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    (tasks, open_fds())
}

async fn cycle(session: &Session, server: &TestServer) {
    let mut work = Vec::new();
    for index in 0..REQUESTS {
        let session = session.clone();
        let base = server.url("");
        work.push(tokio::spawn(async move {
            let url = |path: &str| format!("{}{}", base.trim_end_matches('/'), path);
            match index % 6 {
                0 => drop(session.get(url("/ok")).send().await.unwrap().bytes().await),
                1 => {
                    let mut body = session
                        .get(url("/slow"))
                        .stream()
                        .await
                        .unwrap()
                        .into_stream()
                        .unwrap();
                    drop(body.next().await);
                }
                2 => drop(
                    session
                        .get(url("/slow"))
                        .timeout(TimeoutConfig::new().body(Duration::from_millis(100)))
                        .stream()
                        .await
                        .unwrap()
                        .bytes()
                        .await,
                ),
                3 => drop(session.get(url("/error")).error_for_status().send().await),
                4 => {
                    let short = futures_util::stream::iter([Ok::<_, std::io::Error>(
                        Bytes::from_static(b"short"),
                    )]);
                    drop(
                        session
                            .post(url("/ok"))
                            .body(Body::stream(short, Some(64)))
                            .send()
                            .await,
                    );
                }
                _ => drop(
                    tokio::time::timeout(
                        Duration::from_millis(50),
                        session.get(url("/stall")).send(),
                    )
                    .await,
                ),
            }
        }));
    }
    for task in work {
        task.await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "soak qualification: run with --ignored"]
async fn mixed_failures_return_tasks_and_sockets_to_their_baseline() {
    let server = TestServer::https(reply).await.unwrap();
    let session = Session::builder()
        .tls_trust(server.trust())
        .host_limits(HostLimits::new().max_in_flight(16))
        .build()
        .unwrap();
    cycle(&session, &server).await;
    let (base_tasks, base_fds) = settle().await;
    let mut peak = (base_tasks, base_fds);
    for _ in 1..CYCLES {
        cycle(&session, &server).await;
        let (tasks, fds) = settle().await;
        peak = (peak.0.max(tasks), peak.1.max(fds));
    }
    let (tasks, fds) = settle().await;
    eprintln!(
        "soak: {CYCLES} cycles x {REQUESTS} requests; baseline tasks={base_tasks} fds={base_fds}; \
         peak tasks={} fds={}; final tasks={tasks} fds={fds}",
        peak.0, peak.1
    );
    assert!(
        tasks <= base_tasks + SLACK_TASKS,
        "tasks grew from {base_tasks} to {tasks}"
    );
    assert!(
        fds <= base_fds + SLACK_FDS,
        "open sockets grew from {base_fds} to {fds}"
    );
    assert!(session.host_stats().iter().all(|s| s.in_flight() == 0));
}
