use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use leyline::cookie::{Cookie, Jar};
use leyline::http::HeaderValue;
use leyline::testing::{self, queue};
use leyline::{Body, Kind, Retry, Session, Url};

type Check = Result<(), Box<dyn std::error::Error>>;

fn ok(body: &'static str) -> testing::Response {
    testing::Response::new(200).body(body)
}

#[tokio::test]
async fn a_hook_stamps_an_id_and_a_body_signature_on_every_attempt() -> Check {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(503),
        ok("done"),
        ok("again"),
    ]))
    .await?;
    let ids = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&ids);
    let session = Session::builder()
        .retry(Retry::new(1).any_method(true).when(|attempt| {
            attempt
                .status()
                .is_some_and(|status| status.as_u16() == 503)
        }))
        .before_send(move |request| {
            let id = counter.fetch_add(1, Ordering::SeqCst) + 1;
            let body = request.body().as_bytes().map_or(0, <[u8]>::len);
            let signature = format!("{} {} {body}", request.method(), request.uri().path());
            let headers = request.headers_mut();
            headers.insert("x-request-id", HeaderValue::from(id));
            headers.insert("x-signature", HeaderValue::try_from(signature)?);
            Ok(())
        })
        .build()?;

    let first = session.post(server.url("/sign")).body("hello").await?;
    let second = session.get(server.url("/plain")).await?;

    assert_eq!(first.status(), 200);
    assert_eq!(second.status(), 200);
    let seen = server.requests().await;
    let stamps: Vec<(&str, &str)> = seen
        .iter()
        .map(|request| {
            (
                request.header("x-request-id").unwrap_or("missing"),
                request.header("x-signature").unwrap_or("missing"),
            )
        })
        .collect();
    assert_eq!(
        stamps,
        [
            ("1", "POST /sign 5"),
            ("2", "POST /sign 5"),
            ("3", "GET /plain 0"),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn a_built_request_can_be_read_and_sent_back_through_the_session() -> Check {
    let server = testing::Server::http(|_| ok("built")).await?;
    let session = Session::new();

    let request = session
        .post(server.url("/items"))
        .query([("page", "2")])
        .header("x-trace", "t1")
        .json(&serde_json::json!({ "name": "ada" }))
        .build()?;

    assert_eq!(request.method(), "POST");
    assert_eq!(
        request.uri().path_and_query().map(|p| p.as_str()),
        Some("/items?page=2")
    );
    assert_eq!(
        request.headers().get("x-trace").map(HeaderValue::as_bytes),
        Some(&b"t1"[..])
    );
    assert_eq!(request.body().as_bytes(), Some(&br#"{"name":"ada"}"#[..]));

    let response = session.request_from(request).stream().send().await?;
    let mut body = response.into_http()?.into_body();
    let mut text = Vec::new();
    while let Some(chunk) = body.next().await {
        text.extend_from_slice(&chunk?);
    }

    assert_eq!(text, b"built");
    let seen = server.requests().await;
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].target, "/items?page=2");
    assert_eq!(seen[0].header("x-trace"), Some("t1"));
    assert_eq!(seen[0].header("content-type"), Some("application/json"));
    assert_eq!(seen[0].body, br#"{"name":"ada"}"#);
    Ok(())
}

#[tokio::test]
async fn one_request_is_cloned_and_sent_twice_but_a_streamed_one_is_not() -> Check {
    let server = testing::Server::http(|_| ok("copy")).await?;
    let session = Session::new();

    let request = session
        .put(server.url("/doc"))
        .header("x-copy", "yes")
        .body("payload");
    let copy = request.try_clone().ok_or("a buffered request must clone")?;
    let (first, second) = tokio::join!(request.send(), copy.send());

    assert_eq!(first?.status(), 200);
    assert_eq!(second?.status(), 200);
    let seen = server.requests().await;
    let sent: Vec<(&str, Option<&str>, &[u8])> = seen
        .iter()
        .map(|request| {
            (
                request.method.as_str(),
                request.header("x-copy"),
                request.body.as_slice(),
            )
        })
        .collect();
    assert_eq!(sent, [("PUT", Some("yes"), &b"payload"[..]); 2]);

    let chunks = futures_util::stream::iter([Ok::<_, std::io::Error>(bytes::Bytes::from("x"))]);
    let streamed = session
        .put(server.url("/doc"))
        .body(Body::stream(chunks, None));
    assert!(streamed.try_clone().is_none());
    Ok(())
}

#[tokio::test]
async fn a_file_is_sent_as_the_request_body_with_its_length() -> Check {
    let server = testing::Server::http(|_| ok("stored")).await?;
    let path = std::env::temp_dir().join(format!("leyline-body-file-{}.bin", std::process::id()));
    let content = b"0123456789".repeat(20_000);
    std::fs::write(&path, &content)?;

    let response = Session::new()
        .put(server.url("/upload"))
        .body(Body::file(&path)?)
        .await;
    std::fs::remove_file(&path)?;

    assert_eq!(response?.status(), 200);
    let seen = server.requests().await;
    assert_eq!(seen[0].header("content-length"), Some("200000"));
    assert_eq!(seen[0].body, content);
    Ok(())
}

#[tokio::test]
async fn session_defaults_set_the_user_agent_one_header_and_basic_credentials() -> Check {
    let server = testing::Server::http(|_| ok("hi")).await?;
    let session = Session::builder()
        .user_agent("acme-bot/1.0")
        .header("x-api-key", "k123")
        .basic_auth("ada", "pw")
        .build()?;

    session.get(server.url("/")).await?;

    let seen = server.requests().await;
    assert_eq!(seen[0].header_values("user-agent"), ["acme-bot/1.0"]);
    assert_eq!(seen[0].header("x-api-key"), Some("k123"));
    assert_eq!(seen[0].header("authorization"), Some("Basic YWRhOnB3"));
    Ok(())
}

#[tokio::test]
async fn a_session_without_cookies_ignores_set_cookie() -> Check {
    let server =
        testing::Server::http(|_| ok("hi").header("set-cookie", "sid=abc; Path=/")).await?;
    let session = Session::builder().cookies(None).build()?;

    session.get(server.url("/login")).await?;
    session.get(server.url("/me")).await?;

    let seen = server.requests().await;
    assert_eq!(seen[1].header("cookie"), None);
    assert!(session.cookies().all().is_empty());
    Ok(())
}

#[tokio::test]
async fn a_hand_built_cookie_is_sent_only_where_its_fields_allow() -> Check {
    let server = testing::Server::http(|_| ok("hi")).await?;
    let origin = Url::parse(&server.url("/"))?;
    let jar = Jar::new();
    let mut cookie = Cookie::new("sid", "abc");
    cookie.path = "/app".to_owned();
    cookie.http_only = true;
    jar.insert(&origin, cookie);
    jar.store(&origin, "theme=dark; Path=/");
    let session = Session::builder().cookies(jar.clone()).build()?;

    session.get(server.url("/app/home")).await?;
    session.get(server.url("/other")).await?;

    let seen = server.requests().await;
    let mut inside: Vec<&str> = seen[0].header("cookie").unwrap_or("").split("; ").collect();
    inside.sort_unstable();
    assert_eq!(inside, ["sid=abc", "theme=dark"]);
    assert_eq!(seen[1].header("cookie"), Some("theme=dark"));
    let stored = jar.all();
    let sid = stored
        .iter()
        .find(|cookie| cookie.name == "sid")
        .ok_or("sid is stored")?;
    assert_eq!(
        (sid.domain.as_str(), sid.host_only, sid.http_only),
        ("127.0.0.1", true, true)
    );
    Ok(())
}

#[tokio::test]
async fn events_resume_from_a_given_event_id() -> Check {
    let server = testing::Server::http(|_| {
        testing::Response::new(200)
            .header("content-type", "text/event-stream")
            .body("id: 43\ndata: next\n\n")
    })
    .await?;

    let mut events = Session::new()
        .get(server.url("/events"))
        .events()?
        .last_event_id("42");
    let event = events.next().await.ok_or("one event arrives")??;

    assert_eq!((event.id(), event.data()), ("43", "next"));
    let seen = server.requests().await;
    assert_eq!(seen[0].header("last-event-id"), Some("42"));
    Ok(())
}

#[tokio::test]
async fn the_mock_request_text_is_its_body() -> Check {
    let server = testing::Server::http(|request| {
        let fields = request.text().into_owned();
        let head = request.head().into_owned();
        ok("hi")
            .header("x-fields", fields)
            .header("x-head-starts", head[..11].to_owned())
    })
    .await?;

    let response = Session::new()
        .post(server.url("/login"))
        .form([("user", "ada"), ("pass", "pw")])
        .await?;

    assert_eq!(response.header("x-fields"), Some("user=ada&pass=pw"));
    assert_eq!(response.header("x-head-starts"), Some("POST /login"));
    Ok(())
}

#[tokio::test]
async fn a_body_cut_by_the_mock_server_fails_the_read() -> Check {
    let server = testing::Server::http(|_| {
        testing::Response::new(200)
            .body("0123456789")
            .abort_after(4)
    })
    .await?;
    let session = Session::builder().retry(0).build()?;

    let cut = session.get(server.url("/cut")).text().await;

    let error = cut.err().ok_or("a cut body must not read as complete")?;
    assert!(matches!(error.kind(), Kind::Body | Kind::Io), "{error:?}");
    Ok(())
}

#[test]
fn a_blocking_session_reaches_a_mock_server_started_on_its_own_runtime() -> Check {
    let session = leyline::blocking::Session::new(Session::builder())?;
    let server = session.block_on(testing::Server::http(|request| {
        testing::Response::new(200).body(request.body.clone())
    }))?;

    let text = session.block_on(session.post(server.url("/echo")).body("ping").text())?;

    assert_eq!(text, "ping");
    Ok(())
}
