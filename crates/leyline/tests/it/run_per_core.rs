use std::rc::Rc;

use leyline::Session;
use leyline::testing::{Response, Server};

#[tokio::test(flavor = "multi_thread")]
async fn every_core_fetches_with_its_own_session() {
    let server = Server::http(|_| Response::new(200).body("ok"))
        .await
        .expect("server starts");
    let url = format!("http://{}/", server.addr());
    let bodies = tokio::task::spawn_blocking(move || {
        Session::builder().run_per_core(|session| {
            let url = Rc::new(url.clone());
            async move { session.get(url.as_str()).send().await?.text().await }
        })
    })
    .await
    .expect("runner thread")
    .expect("every session builds");
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    assert_eq!(bodies.len(), cores);
    for body in bodies {
        assert_eq!(body.expect("request succeeds"), "ok");
    }
}

#[test]
fn a_builder_error_is_returned_before_any_work_runs() {
    let ran = std::sync::atomic::AtomicBool::new(false);
    let err = Session::builder()
        .proxy("not a proxy url")
        .run_per_core(|_| async { ran.store(true, std::sync::atomic::Ordering::Relaxed) })
        .unwrap_err();
    assert!(
        err.is_proxy() || err.kind() == leyline::Kind::Config,
        "{err:?}"
    );
    assert!(!ran.load(std::sync::atomic::Ordering::Relaxed));
}

#[tokio::test(flavor = "multi_thread")]
async fn coarse_timeout_fires_after_sibling_cores_finish() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use leyline::{Kind, Timeout};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 512];
                let Ok(n) = stream.read(&mut buf).await else {
                    return;
                };
                if buf[..n].starts_with(b"GET /fast") {
                    drop(
                        stream
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                            .await,
                    );
                } else {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            });
        }
    });

    let counter = Arc::new(AtomicUsize::new(0));
    let url_base = format!("http://{addr}");
    let results = tokio::task::spawn_blocking(move || {
        Session::builder()
            .retry(0)
            .timeout(Timeout::new().total(Duration::from_millis(2500)))
            .run_per_core(move |session| {
                let id = counter.fetch_add(1, Ordering::SeqCst);
                let url = if id == 0 {
                    format!("{url_base}/hang")
                } else {
                    format!("{url_base}/fast")
                };
                async move {
                    if id == 0 {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                    }
                    (id, session.get(&url).text().await)
                }
            })
    })
    .await
    .expect("runner thread")
    .expect("every session builds");

    for (id, res) in results {
        if id == 0 {
            let err = res.expect_err("hanging core must hit 2s coarse timeout");
            assert_eq!(err.kind(), Kind::Timeout, "{err:?}");
        } else {
            assert_eq!(res.expect("fast core succeeds"), "ok");
        }
    }
}
