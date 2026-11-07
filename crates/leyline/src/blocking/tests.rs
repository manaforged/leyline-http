use super::Session;
use crate::testing::{Response, Server};

fn serve() -> (tokio::runtime::Runtime, Server) {
    let runtime = tokio::runtime::Runtime::new().expect("server runtime");
    let server = runtime
        .block_on(Server::https(|_| Response::new(200).body("hello")))
        .expect("server starts");
    (runtime, server)
}

#[test]
fn requests_run_on_the_calling_thread_without_a_runtime() {
    let (_runtime, server) = serve();
    let session = Session::new(crate::Session::builder().tls(server.tls())).expect("session");

    for _ in 0..3 {
        let response = session
            .block_on(session.get(server.url("/")).send())
            .expect("response");
        assert_eq!(response.status(), 200);
        assert_eq!(session.block_on(response.text()).expect("body"), "hello");
    }

    let stats = session.stats();
    assert_eq!(
        stats.misses(http::Version::HTTP_11) + stats.misses(http::Version::HTTP_2),
        1
    );
}

#[test]
fn a_session_moves_to_another_thread_with_its_connections() {
    let (_runtime, server) = serve();
    let session = Session::new(crate::Session::builder().tls(server.tls())).expect("session");
    let url = server.url("/");
    session
        .block_on(session.get(url.as_str()).send())
        .expect("first response");

    let session = std::thread::spawn(move || {
        let response = session
            .block_on(session.get(url.as_str()).send())
            .expect("response on the new thread");
        assert_eq!(response.status(), 200);
        session
    })
    .join()
    .expect("thread");

    let stats = session.stats();
    assert_eq!(
        stats.misses(http::Version::HTTP_11) + stats.misses(http::Version::HTTP_2),
        1
    );
}

fn serve_with(response: Response) -> (tokio::runtime::Runtime, Server) {
    let runtime = tokio::runtime::Runtime::new().expect("server runtime");
    let server = runtime
        .block_on(Server::https(move |_| response.clone()))
        .expect("server starts");
    (runtime, server)
}

#[test]
fn a_total_timeout_ends_a_request_the_server_stalls() {
    let late = Response::new(200)
        .delay(std::time::Duration::from_secs(3))
        .body("late");
    let (_runtime, server) = serve_with(late);
    let session = Session::new(
        crate::Session::builder()
            .protocol(crate::Protocol::Http1)
            .tls(server.tls())
            .timeout(crate::Timeout::new().total(std::time::Duration::from_millis(300))),
    )
    .expect("session");

    let started = std::time::Instant::now();
    let error = session
        .block_on(session.get(server.url("/")).send())
        .expect_err("the server stalls past the deadline");

    assert_eq!(error.kind(), crate::Kind::Timeout);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn a_request_timeout_shorter_than_the_session_timeout_wins() {
    let late = Response::new(200)
        .delay(std::time::Duration::from_secs(3))
        .body("late");
    let (_runtime, server) = serve_with(late);
    let session = Session::new(
        crate::Session::builder()
            .protocol(crate::Protocol::Http1)
            .tls(server.tls()),
    )
    .expect("session");

    let started = std::time::Instant::now();
    let error = session
        .block_on(
            session
                .get(server.url("/"))
                .timeout(crate::Timeout::new().total(std::time::Duration::from_millis(300)))
                .send(),
        )
        .expect_err("the request deadline passes first");

    assert_eq!(error.kind(), crate::Kind::Timeout);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn an_idle_connection_is_checked_and_reused() {
    let (_runtime, server) = serve();
    let session = Session::new(
        crate::Session::builder()
            .protocol(crate::Protocol::Http1)
            .tls(server.tls()),
    )
    .expect("session");

    for _ in 0..2 {
        let response = session
            .block_on(session.get(server.url("/")).send())
            .expect("response");
        assert_eq!(session.block_on(response.text()).expect("body"), "hello");
        session.block_on(async { tokio::time::sleep(std::time::Duration::from_millis(120)).await });
    }

    assert_eq!(session.stats().misses(http::Version::HTTP_11), 1);
}

#[test]
fn plain_http_and_streamed_bodies_work() {
    let runtime = tokio::runtime::Runtime::new().expect("server runtime");
    let server = runtime
        .block_on(Server::http(|_| {
            Response::new(200).body(vec![7u8; 100_000])
        }))
        .expect("server starts");
    let session = Session::new(crate::Session::builder()).expect("session");

    for _ in 0..2 {
        let body = session
            .block_on(async {
                session
                    .get(server.url("/"))
                    .stream()
                    .send()
                    .await?
                    .bytes()
                    .await
            })
            .expect("streamed body");
        assert_eq!(body.len(), 100_000);
        assert!(body.iter().all(|&b| b == 7));
    }
    assert_eq!(session.stats().misses(http::Version::HTTP_11), 1);
}

fn serve_then_drop_each_connection() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}/", listener.local_addr().expect("address"));
    let accepted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = std::sync::Arc::clone(&accepted);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                if stream.read(&mut byte).unwrap_or(0) == 0 {
                    break;
                }
                request.push(byte[0]);
            }
            let length = String::from_utf8_lossy(&request)
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            let mut body = vec![0u8; length];
            drop(stream.read_exact(&mut body));
            drop(stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok"));
        }
    });
    (url, accepted)
}

#[test]
fn a_post_after_the_server_dropped_a_fresh_connection_redials() {
    let (url, accepted) = serve_then_drop_each_connection();
    let session = Session::new(crate::Session::builder()).expect("session");

    let first = session
        .block_on(async { session.get(url.as_str()).send().await?.text().await })
        .expect("first response");
    assert_eq!(first, "ok");
    session.block_on(async { tokio::time::sleep(std::time::Duration::from_millis(20)).await });

    let posted = session
        .block_on(async {
            session
                .post(url.as_str())
                .body("x")
                .send()
                .await?
                .text()
                .await
        })
        .expect("the post goes to a fresh connection");
    assert_eq!(posted, "ok");
    assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[test]
fn a_panic_inside_a_deadline_leaves_no_expiry_behind() {
    let at = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    let panicked = std::panic::catch_unwind(|| {
        crate::core::deadline::expiring(Some(at), || panic!("inside the deadline"))
    });

    assert!(panicked.is_err());
    assert_eq!(crate::core::deadline::expiry(), None);
}

#[test]
fn a_request_body_reaches_the_server_on_a_blocking_connection() {
    let runtime = tokio::runtime::Runtime::new().expect("server runtime");
    let server = runtime
        .block_on(Server::https(|request| {
            Response::new(200).body(request.body.len().to_string())
        }))
        .expect("server starts");
    let session = Session::new(
        crate::Session::builder()
            .protocol(crate::Protocol::Http1)
            .tls(server.tls()),
    )
    .expect("session");

    for _ in 0..2 {
        let echoed = session
            .block_on(async {
                session
                    .put(server.url("/"))
                    .body(vec![b'a'; 100_000])
                    .send()
                    .await?
                    .text()
                    .await
            })
            .expect("upload");
        assert_eq!(echoed, "100000");
    }
    assert_eq!(session.stats().misses(http::Version::HTTP_11), 1);
}

fn serve_slowly(body: usize, gap: std::time::Duration) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}/", listener.local_addr().expect("address"));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            std::thread::spawn(move || {
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        return;
                    }
                    request.push(byte[0]);
                }
                let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {body}\r\n\r\n");
                if stream.write_all(head.as_bytes()).is_err() {
                    return;
                }
                for _ in 0..body {
                    pause(gap);
                    if stream.write_all(b"x").is_err() {
                        return;
                    }
                }
                pause(std::time::Duration::from_secs(5));
            });
        }
    });
    url
}

#[test]
fn a_total_timeout_ends_a_body_the_server_trickles() {
    let url = serve_slowly(10, std::time::Duration::from_millis(150));
    let session = Session::new(
        crate::Session::builder()
            .timeout(crate::Timeout::new().total(std::time::Duration::from_millis(400))),
    )
    .expect("session");

    let started = std::time::Instant::now();
    let error = session
        .block_on(async { session.get(url.as_str()).send().await?.text().await })
        .expect_err("the body arrives after the total deadline");

    assert_eq!(error.kind(), crate::Kind::Timeout);
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

#[test]
fn a_held_stream_from_a_silent_server_does_not_stall_the_next_request() {
    let silent = serve_slowly(10, std::time::Duration::from_secs(60));
    let (fast, _) = serve_then_drop_each_connection();
    let session = Session::new(
        crate::Session::builder()
            .timeout(crate::Timeout::new().total(std::time::Duration::from_secs(4))),
    )
    .expect("session");
    let held = session
        .block_on(session.get(silent.as_str()).stream().send())
        .expect("the head arrives");

    let started = std::time::Instant::now();
    let answered = session.block_on(async {
        session
            .get(fast.as_str())
            .timeout(crate::Timeout::new().total(std::time::Duration::from_millis(500)))
            .send()
            .await?
            .text()
            .await
    });

    assert_eq!(answered.expect("the fast server answers"), "ok");
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    drop(held);
}

fn pause(wait: std::time::Duration) {
    let (_keep, rx) = std::sync::mpsc::channel::<()>();
    let _elapsed = rx.recv_timeout(wait);
}
