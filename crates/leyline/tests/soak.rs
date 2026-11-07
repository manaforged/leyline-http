#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::StreamExt;
use leyline::testing;
use leyline::{Body, HostLimits, Kind, Session, Timeout};

const CYCLES: usize = 12;
const REQUESTS: usize = 60;
const SLACK_TASKS: usize = 4;
const SLACK_FDS: usize = 8;

#[derive(Clone, Copy)]
enum Scenario {
    Complete,
    Abandon,
    BodyTimeout,
    StatusError,
    ShortUpload,
    Cancel,
}

const SCENARIOS: [Scenario; 6] = [
    Scenario::Complete,
    Scenario::Abandon,
    Scenario::BodyTimeout,
    Scenario::StatusError,
    Scenario::ShortUpload,
    Scenario::Cancel,
];

fn reply(request: &testing::Request) -> testing::Response {
    match request.target.as_str() {
        "/ok" => testing::Response::new(200).body(vec![b'o'; 2048]),
        "/slow" => {
            testing::Response::new(200).chunks(["a", "b", "c", "d"], Duration::from_millis(150))
        }
        "/error" => testing::Response::new(503).body(vec![b'e'; 512]),
        "/stall" => testing::Response::new(200).delay(Duration::from_millis(300)),
        _ => testing::Response::new(404),
    }
}

fn open_fds() -> Option<usize> {
    #[cfg(unix)]
    {
        let dir = if cfg!(target_os = "linux") {
            "/proc/self/fd"
        } else {
            "/dev/fd"
        };
        Some(
            std::fs::read_dir(dir)
                .expect("file descriptor probe")
                .count(),
        )
    }
    #[cfg(not(unix))]
    {
        None
    }
}

async fn settle() -> (usize, Option<usize>) {
    tokio::time::sleep(Duration::from_millis(500)).await;
    let tasks = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    (tasks, open_fds())
}

async fn complete(session: &Session, server: &testing::Server) {
    let response = session.get(server.url("/ok")).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.bytes().await.unwrap().as_ref(), vec![b'o'; 2048]);
}

async fn abandon(session: &Session, server: &testing::Server) {
    let response = session
        .get(server.url("/slow"))
        .stream()
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let mut body = response.bytes_stream().unwrap();
    assert_eq!(body.next().await.unwrap().unwrap().as_ref(), b"a");
    drop(body);
}

async fn body_timeout(session: &Session, server: &testing::Server) {
    let error = session
        .get(server.url("/slow"))
        .timeout(Timeout::new().body(Duration::from_millis(100)))
        .stream()
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap_err();
    assert_eq!(error.kind(), Kind::Timeout, "{error:?}");
}

async fn status_error(session: &Session, server: &testing::Server) {
    let error = session
        .get(server.url("/error"))
        .error_for_status()
        .send()
        .await
        .unwrap_err();
    assert_eq!(error.kind(), Kind::Status);
    assert_eq!(error.status().map(|status| status.as_u16()), Some(503));
    assert_eq!(error.body().unwrap(), vec![b'e'; 512]);
}

async fn short_upload(session: &Session, server: &testing::Server) {
    let short = futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from_static(b"short"))]);
    let error = session
        .post(server.url("/ok"))
        .body(Body::stream(short, Some(64)))
        .send()
        .await
        .unwrap_err();
    assert_eq!(error.kind(), Kind::Body, "{error:?}");
}

async fn cancel(session: &Session, server: &testing::Server) {
    tokio::time::timeout(
        Duration::from_millis(50),
        session.get(server.url("/stall")).send(),
    )
    .await
    .expect_err("cancel before the delayed response");
}

async fn cycle(session: &Session, server: &Arc<testing::Server>) {
    let mut work = Vec::new();
    for scenario in SCENARIOS.into_iter().cycle().take(REQUESTS) {
        let (session, server) = (session.clone(), server.clone());
        work.push(tokio::spawn(async move {
            match scenario {
                Scenario::Complete => complete(&session, &server).await,
                Scenario::Abandon => abandon(&session, &server).await,
                Scenario::BodyTimeout => body_timeout(&session, &server).await,
                Scenario::StatusError => status_error(&session, &server).await,
                Scenario::ShortUpload => short_upload(&session, &server).await,
                Scenario::Cancel => cancel(&session, &server).await,
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
    let server = Arc::new(testing::Server::https(reply).await.unwrap());
    let session = Session::builder()
        .tls(server.tls())
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
    let deadline = Instant::now() + Duration::from_secs(10);
    let (tasks, fds) = loop {
        let (tasks, fds) = settle().await;
        let sockets_back = match (base_fds, fds) {
            (Some(base), Some(current)) => current <= base + SLACK_FDS,
            _ => true,
        };
        if (tasks <= base_tasks + SLACK_TASKS && sockets_back) || Instant::now() >= deadline {
            break (tasks, fds);
        }
    };
    eprintln!(
        "soak: {CYCLES} cycles x {REQUESTS} requests; baseline tasks={base_tasks} fds={base_fds:?}; \
         peak tasks={} fds={:?}; final tasks={tasks} fds={fds:?}",
        peak.0, peak.1
    );
    assert!(
        tasks <= base_tasks + SLACK_TASKS,
        "tasks grew from {base_tasks} to {tasks}"
    );
    if let (Some(base), Some(current)) = (base_fds, fds) {
        assert!(
            current <= base + SLACK_FDS,
            "open sockets grew from {base} to {current}"
        );
    } else {
        eprintln!("file descriptor measurement unavailable on this platform");
    }
    assert!({
        let stats = session.stats();
        stats.hosts().all(|h| stats.in_flight(h) == 0)
    });
}
