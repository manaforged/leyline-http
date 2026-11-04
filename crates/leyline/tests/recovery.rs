#![expect(
    clippy::unwrap_used,
    reason = "test harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use leyline::testing::{Request, Response, Server};
use leyline::{Protocol, Proxy, Retry, Session};

const BODY_LEN: usize = 64 * 1024;
const ETAG: &str = "\"v1\"";

fn builder() -> leyline::SessionBuilder {
    Session::builder()
        .protocol(Protocol::Http1)
        .proxy(Proxy::new().env(false))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("leyline-recovery-{name}-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn body() -> Vec<u8> {
    (0..BODY_LEN).map(|i| (i % 251) as u8).collect()
}

#[tokio::test]
async fn a_fresh_response_is_served_from_the_cache_until_cleared() {
    let server = Server::http(|_: &Request| {
        Response::new(200)
            .header("cache-control", "max-age=60")
            .body("hello")
    })
    .await
    .unwrap();
    let session = builder().cache(1 << 20).build().unwrap();
    let url = server.url("/fresh");
    for _ in 0..2 {
        let text = session
            .get(&url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(text, "hello");
    }
    assert_eq!(server.requests().await.len(), 1);
    session.clear_cache();
    session.get(&url).send().await.unwrap();
    assert_eq!(server.requests().await.len(), 2);
}

#[tokio::test]
async fn a_stale_response_is_revalidated_with_its_etag() {
    let server = Server::http(|request: &Request| {
        let base = Response::new(200)
            .header("cache-control", "no-cache")
            .header("etag", ETAG);
        match request.header("if-none-match") {
            Some(ETAG) => Response::new(304)
                .header("cache-control", "no-cache")
                .header("etag", ETAG),
            _ => base.body("hello"),
        }
    })
    .await
    .unwrap();
    let session = builder().cache(1 << 20).build().unwrap();
    let url = server.url("/etag");
    for _ in 0..2 {
        let response = session.get(&url).send().await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.unwrap(), "hello");
    }
    let seen: Vec<_> = server
        .requests()
        .await
        .iter()
        .map(|request| request.header("if-none-match").map(str::to_owned))
        .collect();
    assert_eq!(seen, [None, Some(ETAG.to_owned())]);
}

#[tokio::test]
async fn an_interrupted_download_resumes_after_the_server_sets_a_cookie() {
    let full = body();
    let half = BODY_LEN / 2;
    let served = full.clone();
    let server = Server::http(move |request: &Request| match request.header("range") {
        Some(range)
            if range == format!("bytes={half}-") && request.header("if-range") == Some(ETAG) =>
        {
            Response::new(206)
                .header("etag", ETAG)
                .header(
                    "content-range",
                    format!("bytes {half}-{}/{BODY_LEN}", BODY_LEN - 1),
                )
                .body(served[half..].to_vec())
        }
        _ => Response::new(200)
            .header("etag", ETAG)
            .header("set-cookie", "session=1; Path=/")
            .header("content-length", BODY_LEN.to_string())
            .body(served[..half].to_vec())
            .close(),
    })
    .await
    .unwrap();
    let session = builder().build().unwrap();
    let path = scratch("download").join("file.bin");
    let url = server.url("/file");
    session.get(&url).download_to(&path).await.unwrap_err();
    assert!(!path.exists());
    let written = session.get(&url).download_to(&path).await.unwrap();
    assert_eq!(written, BODY_LEN as u64);
    assert_eq!(std::fs::read(&path).unwrap(), full);
    let ranges: Vec<_> = server
        .requests()
        .await
        .iter()
        .map(|request| request.header("range").map(str::to_owned))
        .collect();
    assert_eq!(ranges, [None, Some(format!("bytes={half}-"))]);
}

fn resumable_server(full: Vec<u8>) -> impl Fn(&Request) -> Response + Send + Sync + 'static {
    let half = BODY_LEN / 2;
    move |request: &Request| match request.header("range") {
        Some(range) if range == format!("bytes={half}-") => Response::new(206)
            .header("etag", ETAG)
            .header(
                "content-range",
                format!("bytes {half}-{}/{BODY_LEN}", BODY_LEN - 1),
            )
            .body(full[half..].to_vec()),
        _ => Response::new(200)
            .header("etag", ETAG)
            .header("content-length", BODY_LEN.to_string())
            .body(full[..half].to_vec())
            .close(),
    }
}

#[tokio::test]
async fn a_resumed_download_checks_the_sha256_of_the_whole_file() {
    use sha2::{Digest, Sha256};

    let full = body();
    let good: [u8; 32] = Sha256::digest(&full).into();
    for (checksum, accepted) in [(good, true), ([7; 32], false)] {
        let server = Server::http(resumable_server(full.clone())).await.unwrap();
        let session = builder().build().unwrap();
        let path = scratch(&format!("sha-{accepted}")).join("file.bin");
        let url = server.url("/file");
        session
            .get(&url)
            .sha256(checksum)
            .download_to(&path)
            .await
            .unwrap_err();
        let resumed = session.get(&url).sha256(checksum).download_to(&path).await;
        assert_eq!(resumed.is_ok(), accepted, "{resumed:?}");
        assert_eq!(std::fs::read(&path).ok(), accepted.then(|| full.clone()));
        let ranges = server.requests().await;
        assert_eq!(
            ranges.last().unwrap().header("range"),
            Some(format!("bytes={}-", BODY_LEN / 2).as_str())
        );
    }
}

#[tokio::test]
async fn a_complete_prefix_of_unknown_length_finishes_on_416() {
    let server = Server::http(|request: &Request| match request.header("range") {
        Some("bytes=5-") if request.header("if-range") == Some(ETAG) => Response::new(416)
            .header("content-range", "bytes */5")
            .body("range not satisfiable"),
        _ => Response::new(200)
            .header("etag", ETAG)
            .header("transfer-encoding", "chunked")
            .body("5\r\nhello\r\n")
            .close(),
    })
    .await
    .unwrap();
    let session = builder().build().unwrap();
    let path = scratch("prefix-416").join("file.bin");
    let url = server.url("/file");
    session.get(&url).download_to(&path).await.unwrap_err();
    let written = session.get(&url).download_to(&path).await.unwrap();
    assert_eq!(written, 5);
    assert_eq!(std::fs::read(&path).unwrap(), b"hello");
}

struct SlowResolver {
    addr: std::net::SocketAddr,
    calls: Arc<AtomicUsize>,
}

impl leyline::Resolver for SlowResolver {
    fn resolve<'a>(
        &'a self,
        _host: &'a str,
        port: u16,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = std::io::Result<Vec<std::net::SocketAddr>>> + Send + 'a>,
    > {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let addr = std::net::SocketAddr::new(self.addr.ip(), port);
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok(vec![addr])
        })
    }
}

#[tokio::test]
async fn concurrent_cold_requests_share_one_dns_lookup() {
    let server = Server::http(|_: &Request| Response::new(200).body("ok"))
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let session = builder()
        .dns_resolver(Arc::new(SlowResolver {
            addr: server.addr(),
            calls: Arc::clone(&calls),
        }))
        .build()
        .unwrap();
    let url = format!("http://burst.test:{}/", server.addr().port());
    let requests = (0..8).map(|_| {
        let session = session.clone();
        let url = url.clone();
        async move { session.get(url).send().await.unwrap().text().await.unwrap() }
    });
    let bodies = futures_util::future::join_all(requests).await;
    assert_eq!(bodies, ["ok"; 8]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_event_stream_resumes_from_the_last_id_across_more_than_three_drops() {
    let server = Server::http(|request: &Request| {
        let last: u32 = request
            .header("last-event-id")
            .map_or(0, |id| id.parse().unwrap());
        let next = last + 1;
        Response::new(200)
            .header("content-type", "text/event-stream")
            .body(format!("retry: 1\nid: {next}\ndata: e{next}\n\n"))
            .close()
    })
    .await
    .unwrap();
    let session = builder().build().unwrap();
    let mut events = session
        .get(server.url("/events"))
        .events()
        .unwrap()
        .retry(Retry::new(3).backoff(Duration::from_millis(1), Duration::from_millis(1)));
    let mut seen = Vec::new();
    for _ in 0..6 {
        let event = events.next().await.unwrap().unwrap();
        seen.push((event.id().to_owned(), event.data().to_owned()));
    }
    let expected: Vec<_> = (1..=6).map(|n| (n.to_string(), format!("e{n}"))).collect();
    assert_eq!(seen, expected);
}

#[tokio::test]
async fn concurrent_401s_share_one_token_refresh() {
    let server = Server::http(|request: &Request| match request.header("authorization") {
        Some("Bearer new") => Response::new(200).body("ok"),
        _ => Response::new(401),
    })
    .await
    .unwrap();
    let refreshes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&refreshes);
    let session = builder()
        .base_url(server.url("/"))
        .bearer_auth("old")
        .refresh_token(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok("new".to_owned())
            }
        })
        .build()
        .unwrap();
    let requests = (0..4).map(|_| {
        let session = session.clone();
        async move {
            session
                .get("/data")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap()
        }
    });
    let bodies = futures_util::future::join_all(requests).await;
    assert_eq!(bodies, ["ok"; 4]);
    assert_eq!(refreshes.load(Ordering::SeqCst), 1);
}

#[cfg(feature = "html")]
#[tokio::test]
async fn a_saved_crawl_resumes_without_fetching_a_page_twice() {
    use leyline::HostLimits;

    let server = Server::http(|request: &Request| {
        let html = match request.target.as_str() {
            "/" => r#"<a href="/a">a</a><a href="/b">b</a><a href="/c">c</a>"#,
            _ => r#"<a href="/">home</a>"#,
        };
        Response::new(200)
            .header("content-type", "text/html")
            .body(html)
    })
    .await
    .unwrap();
    let session = builder().build().unwrap();
    let mut crawl = session
        .crawl(server.url("/"))
        .respect(None)
        .host_limits(HostLimits::new());
    let mut pages = vec![crawl.next().await.unwrap().unwrap().url().path().to_owned()];
    let path = scratch("crawl").join("crawl.json");
    crawl.save(&path).await.unwrap();
    drop(crawl);
    let mut resumed = leyline::Crawl::load(&builder().build().unwrap(), &path).unwrap();
    while let Some(page) = resumed.next().await {
        pages.push(page.unwrap().url().path().to_owned());
    }
    pages.sort();
    assert_eq!(pages, ["/", "/a", "/b", "/c"]);
    let mut fetched: Vec<_> = server
        .requests()
        .await
        .into_iter()
        .map(|request| request.target)
        .collect();
    fetched.sort();
    let before = fetched.len();
    fetched.dedup();
    assert_eq!(
        fetched.len(),
        before,
        "a URL was fetched twice: {fetched:?}"
    );
    assert!(
        ["/", "/a", "/b", "/c"]
            .iter()
            .all(|page| fetched.contains(&page.to_string()))
    );
}
