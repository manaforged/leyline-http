use crate::error::Kind;
use crate::testing::{Request, Response, Server};
use crate::{Protocol, Proxy, Session};

const ETAG: &str = "\"v1\"";

const REFUSED: [&str; 19] = [
    "bytes 4-9/10",
    "bytes 6-9/10",
    "bytes 5-8/10",
    "bytes 5-9/11",
    "bytes 5-10/11",
    "bytes 5-4/5",
    "Bytes 5-9/10",
    "bytes5-9/10",
    "5-9/10",
    "bytes 5-9",
    "bytes 5/10",
    "bytes */10",
    "bytes 5-/10",
    "bytes -9/10",
    "bytes 5-9/",
    "bytes +5-9/10",
    "bytes 5-+9/10",
    "bytes 5-9/+10",
    "bytes 5-18446744073709551615/0",
];

fn resumable(content_range: &'static str) -> impl Fn(&Request) -> Response + Send + Sync + 'static {
    move |request: &Request| match request.header("range") {
        Some("bytes=5-") => Response::new(206)
            .header("etag", ETAG)
            .header("content-range", content_range)
            .body("56789"),
        _ => Response::new(200)
            .header("etag", ETAG)
            .header("content-length", "10")
            .body("01234")
            .close(),
    }
}

async fn resume(case: usize, content_range: &'static str) -> (crate::Result<u64>, Option<Vec<u8>>) {
    let server = Server::http(resumable(content_range))
        .await
        .expect("server starts");
    let session = Session::builder()
        .protocol(Protocol::Http1)
        .proxy(Proxy::new().env(false))
        .build()
        .expect("session");
    let dir = std::env::temp_dir().join(format!(
        "leyline-download-range-{}-{case}",
        std::process::id()
    ));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("scratch directory");
    let path = dir.join("file.bin");
    let url = server.url("/file");
    session
        .get(&url)
        .download_to(&path)
        .await
        .expect_err("the first attempt ends five bytes short");
    let resumed = session.get(&url).download_to(&path).await;
    let requests = server.requests().await;
    assert_eq!(
        requests.last().expect("two requests").header("range"),
        Some("bytes=5-"),
        "{content_range}"
    );
    let file = std::fs::read(&path).ok();
    drop(std::fs::remove_dir_all(&dir));
    (resumed, file)
}

#[tokio::test]
async fn a_206_for_the_saved_offset_and_total_resumes() {
    let (resumed, file) = resume(REFUSED.len(), "bytes 5-9/10").await;
    assert_eq!(resumed.expect("resumed download"), 10);
    assert_eq!(file.as_deref(), Some(&b"0123456789"[..]));
}

#[tokio::test]
async fn a_206_with_any_other_content_range_fails_without_a_file() {
    for (case, content_range) in REFUSED.into_iter().enumerate() {
        let (resumed, file) = resume(case, content_range).await;
        let error = resumed.expect_err(content_range);
        assert_eq!(error.kind(), Kind::Body, "{content_range}");
        assert_eq!(file, None, "{content_range}");
    }
}
