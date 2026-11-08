use crate::Session;
use crate::testing::{Request, Response, Server, queue};

#[tokio::test]
async fn crlf_header_fails_before_io() {
    let err = Session::builder()
        .build()
        .unwrap()
        .get("https://example.test/")
        .header("x-a", "1\r\nHost: evil")
        .send()
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("header value"),
        "unexpected: {err}"
    );
}

#[tokio::test]
async fn crlf_url_fails_before_io() {
    let err = Session::builder()
        .build()
        .unwrap()
        .request(http::Method::GET, "https://example.test/\r\nHost: evil")
        .send()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("URL"), "unexpected: {err}");
}

fn loopback() -> crate::SessionBuilder {
    Session::builder()
        .protocol(crate::Protocol::Http1)
        .proxy(crate::Proxy::new().env(false))
}

async fn fetch_headers(
    server: &Server,
    request: crate::RequestBuilder,
) -> (Option<String>, Option<String>) {
    request.send().await.expect("response");
    let seen = server.next_request().await.expect("request");
    (
        seen.header("sec-fetch-mode").map(str::to_owned),
        seen.header("sec-fetch-dest").map(str::to_owned),
    )
}

fn fetch(mode: &str, dest: &str) -> (Option<String>, Option<String>) {
    (Some(mode.to_owned()), Some(dest.to_owned()))
}

#[tokio::test]
async fn content_type_selects_the_fetch_headers_on_the_builder_path() {
    use crate::profile::RequestKind;
    let server = Server::http(|_: &Request| Response::new(200))
        .await
        .expect("server starts");
    let session = loopback()
        .browser(crate::Browser::Chrome154)
        .build()
        .expect("session");
    let url = server.url("/");

    let json = session
        .post(&url)
        .header("content-type", "application/json")
        .body("{}");
    assert_eq!(fetch_headers(&server, json).await, fetch("cors", "empty"));

    let form = session
        .post(&url)
        .header("content-type", "application/x-www-form-urlencoded")
        .body("a=1");
    assert_eq!(fetch_headers(&server, form).await, fetch("cors", "empty"));

    let get = session.get(&url).header("content-type", "application/json");
    assert_eq!(
        fetch_headers(&server, get).await,
        fetch("navigate", "document")
    );

    let pinned = session
        .post(&url)
        .kind(RequestKind::Navigate)
        .header("content-type", "application/json");
    assert_eq!(
        fetch_headers(&server, pinned).await,
        fetch("navigate", "document")
    );
}

async fn unavailable_once() -> Server {
    Server::http(queue([Response::new(503), Response::new(200)]))
        .await
        .expect("server starts")
}

#[tokio::test]
async fn session_retry_default_is_inherited_by_requests() {
    let session = loopback()
        .retry(crate::Retry::new(7))
        .build()
        .expect("session");

    let server = unavailable_once().await;
    let inherited = session.get(server.url("/")).send().await.expect("response");
    assert_eq!(inherited.status(), 200);
    assert_eq!(server.requests().await.len(), 2);

    let server = unavailable_once().await;
    let overridden = session
        .get(server.url("/"))
        .retry(0)
        .send()
        .await
        .expect("response");
    assert_eq!(overridden.status(), 503);
    assert_eq!(server.requests().await.len(), 1);

    let server = unavailable_once().await;
    let bare = loopback()
        .build()
        .expect("session")
        .get(server.url("/"))
        .send()
        .await
        .expect("response");
    assert_eq!(bare.status(), 503);
    assert_eq!(server.requests().await.len(), 1);
}
