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
    assert_eq!(seen[0].target(), "/items?page=2");
    assert_eq!(seen[0].header("x-trace"), Some("t1"));
    assert_eq!(seen[0].header("content-type"), Some("application/json"));
    assert_eq!(seen[0].body(), br#"{"name":"ada"}"#);
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
        .map(|request| (request.method(), request.header("x-copy"), request.body()))
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
    assert_eq!(seen[0].body(), content);
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
    let cookie = Cookie::builder("sid", "abc")
        .path("/app")
        .http_only(true)
        .build();
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
        .find(|cookie| cookie.name() == "sid")
        .ok_or("sid is stored")?;
    assert_eq!(
        (sid.domain(), sid.host_only(), sid.http_only()),
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
        testing::Response::new(200).body(request.body().to_vec())
    }))?;

    let text = session.block_on(session.post(server.url("/echo")).body("ping").text())?;

    assert_eq!(text, "ping");
    Ok(())
}

#[tokio::test]
async fn a_cookie_free_session_stays_cookie_free_after_a_manual_insert_and_a_reload() -> Check {
    let server =
        testing::Server::http(|_| ok("hi").header("set-cookie", "sid=abc; Path=/")).await?;
    let origin = Url::parse(&server.url("/"))?;
    let path = std::env::temp_dir().join(format!("leyline-no-cookies-{}.json", std::process::id()));
    let session = Session::builder().cookies(None).build()?;
    session
        .cookies()
        .insert(&origin, Cookie::new("manual", "1"));

    session.get(server.url("/first")).await?;
    session.save(&path)?;
    let reloaded = Session::load(&path).and_then(leyline::SessionBuilder::build);
    std::fs::remove_file(&path)?;
    let reloaded = reloaded?;
    reloaded.get(server.url("/second")).await?;
    reloaded.get(server.url("/third")).await?;

    let seen = server.requests().await;
    let cookies: Vec<Option<&str>> = seen
        .iter()
        .map(|request| request.header("cookie"))
        .collect();
    assert_eq!(cookies, [None, None, None]);
    assert!(reloaded.cookies().all().is_empty());
    Ok(())
}

#[tokio::test]
async fn basic_credentials_go_only_to_the_base_url_origin() -> Check {
    let home = testing::Server::http(|_| ok("home")).await?;
    let other = testing::Server::http(|_| ok("other")).await?;
    let session = Session::builder()
        .base_url(home.url("/"))
        .basic_auth("ada", "pw")
        .build()?;

    session.get("/inside").await?;
    session.get(other.url("/outside")).await?;

    assert_eq!(
        home.requests().await[0].header("authorization"),
        Some("Basic YWRhOnB3")
    );
    assert_eq!(other.requests().await[0].header("authorization"), None);
    Ok(())
}

#[tokio::test]
async fn a_proxy_closure_routes_one_path_through_a_proxy_and_the_rest_direct() -> Check {
    let direct = testing::Server::http(|_| ok("direct")).await?;
    let proxy = testing::Server::http(|_| ok("proxied")).await?;
    let through = proxy.url("/");
    let session = Session::builder()
        .proxy(leyline::Proxy::custom(move |url| {
            url.path()
                .starts_with("/via")
                .then(|| leyline::Proxy::from(through.as_str()))
        }))
        .build()?;

    let via = session.get(direct.url("/via/a")).await?;
    let straight = session.get(direct.url("/plain")).await?;

    assert!(via.proxy().is_some());
    assert_eq!(straight.proxy(), None);
    assert_eq!(via.text().await?, "proxied");
    assert_eq!(straight.text().await?, "direct");
    let proxied = proxy.requests().await;
    assert_eq!(proxied.len(), 1);
    assert_eq!(proxied[0].target(), direct.url("/via/a"));
    let reached: Vec<String> = direct
        .requests()
        .await
        .into_iter()
        .map(|request| request.target().to_owned())
        .collect();
    assert_eq!(reached, ["/plain"]);
    Ok(())
}

#[tokio::test]
async fn a_response_names_the_address_it_came_from() -> Check {
    let server = testing::Server::http(|_| ok("hi")).await?;
    let session = Session::new();

    let first = session.get(server.url("/a")).await?;
    let second = session.get(server.url("/b")).await?;

    assert_eq!(first.remote_addr(), Some(server.addr()));
    assert!(second.reused());
    assert_eq!(second.remote_addr(), Some(server.addr()));
    Ok(())
}

#[tokio::test]
async fn a_cached_response_says_it_came_from_the_cache() -> Check {
    let server =
        testing::Server::http(|_| ok("fresh").header("cache-control", "max-age=60")).await?;
    let session = Session::builder().cache(1 << 20).build()?;

    let first = session.get(server.url("/doc")).await?;
    let second = session.get(server.url("/doc")).await?;

    assert!(!first.from_cache());
    assert!(second.from_cache());
    assert_eq!(second.remote_addr(), None);
    assert_eq!(second.text().await?, "fresh");
    assert_eq!(server.requests().await.len(), 1);
    Ok(())
}

#[tokio::test]
async fn a_timeout_converts_to_a_timed_out_io_error_that_keeps_its_source() -> Check {
    let server =
        testing::Server::http(|_| ok("late").delay(std::time::Duration::from_secs(5))).await?;
    let session = Session::builder().retry(0).build()?;

    let failed = session
        .get(server.url("/slow"))
        .timeout(std::time::Duration::from_millis(50))
        .await;

    let error = failed.err().ok_or("the request must time out")?;
    let attempts = error.attempts();
    let io = std::io::Error::from(error);
    assert_eq!(io.kind(), std::io::ErrorKind::TimedOut);
    let source = leyline::Error::find(&io).ok_or("the leyline error stays reachable")?;
    assert_eq!(source.kind(), Kind::Timeout);
    let shown = source.to_string();
    let back = leyline::Error::from(io);
    assert_eq!(back.kind(), Kind::Timeout);
    assert_eq!(back.to_string(), shown);
    assert_eq!(back.attempts(), attempts);
    assert!(std::error::Error::source(&back).is_none_or(|source| !source.is::<std::io::Error>()));
    Ok(())
}

#[tokio::test]
async fn one_request_can_force_its_protocol_and_keep_it_through_build() -> Check {
    let server = testing::Server::http(|_| ok("hi")).await?;
    let session = Session::new();

    let request = session
        .get(server.url("/h1"))
        .protocol(leyline::Protocol::Http1)
        .build()?;
    assert_eq!(
        request.extensions().get::<leyline::Protocol>(),
        Some(&leyline::Protocol::Http1)
    );
    let forced = session.request_from(request).await?;
    let refused = session
        .get(server.url("/h3"))
        .protocol(leyline::Protocol::Http3)
        .timeout(std::time::Duration::from_millis(500))
        .await;

    assert_eq!(forced.version(), leyline::http::Version::HTTP_11);
    assert!(
        refused.is_err(),
        "a plain HTTP/1.1 server cannot answer a forced HTTP/3 request"
    );
    assert_eq!(server.requests().await.len(), 1);
    Ok(())
}

#[tokio::test]
async fn a_proxy_closure_runs_once_per_request_and_honors_its_bypass_list() -> Check {
    let direct = testing::Server::http(|_| ok("direct")).await?;
    let proxy = testing::Server::http(|_| ok("proxied")).await?;
    let through = proxy.url("/");
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = std::sync::Arc::clone(&calls);
    let choose = leyline::Proxy::custom(move |_| {
        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Some(leyline::Proxy::from(through.as_str()))
    });
    let session = Session::builder().proxy(choose.clone()).build()?;
    let bypassing = Session::builder()
        .proxy(choose.no_proxy(["127.0.0.1"]))
        .build()?;

    let via = session.get(direct.url("/a")).await?;
    let once = calls.load(std::sync::atomic::Ordering::SeqCst);
    let straight = bypassing.get(direct.url("/b")).await?;

    assert_eq!(once, 1);
    assert!(via.proxy().is_some());
    assert_eq!(straight.proxy(), None);
    assert_eq!(via.text().await?, "proxied");
    assert_eq!(straight.text().await?, "direct");
    Ok(())
}

#[tokio::test]
async fn a_hook_that_removes_one_header_keeps_the_order_of_the_rest() -> Check {
    let server = testing::Server::http(|_| ok("hi")).await?;
    let session = Session::builder()
        .before_send(|request| {
            request.headers_mut().remove("x-first");
            Ok(())
        })
        .build()?;

    session
        .get(server.url("/"))
        .header("x-first", "1")
        .header("x-second", "2")
        .header("x-third", "3")
        .header("x-fourth", "4")
        .await?;

    let seen = server.requests().await;
    let head = seen[0].head().to_ascii_lowercase();
    let at = |name: &str| head.find(name);
    assert_eq!(at("x-first"), None);
    assert!(
        at("x-second") < at("x-third") && at("x-third") < at("x-fourth"),
        "{head}"
    );
    Ok(())
}

#[tokio::test]
async fn a_forced_protocol_skips_the_cache() -> Check {
    let server =
        testing::Server::http(|_| ok("fresh").header("cache-control", "max-age=60")).await?;
    let session = Session::builder().cache(1 << 20).build()?;

    session.get(server.url("/doc")).await?;
    let forced = session
        .get(server.url("/doc"))
        .protocol(leyline::Protocol::Http1)
        .await?;

    assert!(!forced.from_cache());
    assert_eq!(server.requests().await.len(), 2);
    Ok(())
}

#[cfg(feature = "http3")]
#[tokio::test]
async fn one_request_forces_http3_on_a_session_that_defaults_to_tcp() -> Check {
    use crate::h3_support::{Limits, Reply, h3_server};
    let server = h3_server(vec![Reply::Body(4096)], Limits::default()).await;
    let session = server.session().protocol(leyline::Protocol::Auto).build()?;

    let response = session
        .get(server.url())
        .protocol(leyline::Protocol::Http3)
        .await?;

    assert_eq!(response.version(), leyline::http::Version::HTTP_3);
    assert_eq!(response.bytes().await?.len(), 4096);
    Ok(())
}

#[tokio::test]
async fn a_hook_can_sign_the_url_and_change_the_method() -> Check {
    let server = testing::Server::http(|_| ok("hi")).await?;
    let session = Session::builder()
        .before_send(|request| {
            let signed = format!("{}&sig=abc", request.uri());
            *request.uri_mut() = signed.parse()?;
            *request.method_mut() = leyline::http::Method::PUT;
            Ok(())
        })
        .build()?;

    let response = session.post(server.url("/pay?n=1")).body("x").await?;

    let seen = server.requests().await;
    assert_eq!(
        (seen[0].method(), seen[0].target()),
        ("PUT", "/pay?n=1&sig=abc")
    );
    assert_eq!(response.url().query(), Some("n=1&sig=abc"));
    Ok(())
}

#[tokio::test]
async fn a_hook_that_leaves_a_relative_url_fails_the_request() -> Check {
    let server = testing::Server::http(|_| ok("hi")).await?;
    let session = Session::builder()
        .before_send(|request| {
            *request.uri_mut() = leyline::http::Uri::from_static("/only-a-path");
            Ok(())
        })
        .build()?;

    let failed = session.get(server.url("/")).await;

    assert_eq!(failed.err().map(|error| error.kind()), Some(Kind::Url));
    assert!(server.requests().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn a_download_checksum_survives_build_and_request_from() -> Check {
    let server = testing::Server::http(|_| ok("payload")).await?;
    let session = Session::new();
    let dir = std::env::temp_dir().join(format!("leyline-sum-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("file.bin");

    let request = session.get(server.url("/file")).sha256([7; 32]).build()?;
    let failed = session.request_from(request).download_to(&path).await;
    let kept = path.exists();
    drop(std::fs::remove_dir_all(&dir));

    let error = failed
        .err()
        .ok_or("a wrong checksum must fail the download")?;
    assert_eq!(error.kind(), Kind::Body);
    assert!(error.to_string().contains("SHA-256"), "{error}");
    assert!(!kept);
    Ok(())
}

#[tokio::test]
async fn a_request_user_agent_drops_the_browser_client_hints() -> Check {
    let server = testing::Server::http(|_| ok("hi")).await?;
    let session = Session::builder()
        .browser(leyline::Browser::Chrome154)
        .build()?;

    session.get(server.url("/browser")).await?;
    session
        .get(server.url("/bot"))
        .header("user-agent", "acme-bot/1.0")
        .await?;
    session
        .get(server.url("/mixed"))
        .header("user-agent", "acme-bot/1.0")
        .header("sec-ch-ua-platform", "\"Linux\"")
        .await?;

    let seen = server.requests().await;
    let hints = |index: usize| -> Vec<&str> {
        seen[index]
            .headers()
            .iter()
            .map(|(name, _)| name.as_str())
            .filter(|name| name.to_ascii_lowercase().starts_with("sec-ch-ua"))
            .collect()
    };
    assert!(hints(0).len() >= 3, "{:?}", hints(0));
    assert_eq!(seen[1].header("user-agent"), Some("acme-bot/1.0"));
    assert!(hints(1).is_empty(), "{:?}", hints(1));
    assert_eq!(hints(2).len(), 1);
    assert_eq!(seen[2].header("sec-ch-ua-platform"), Some("\"Linux\""));
    Ok(())
}

#[tokio::test]
async fn a_session_header_set_twice_sends_both_values_in_order() -> Check {
    let server = testing::Server::http(|_| ok("hi")).await?;
    let session = Session::builder()
        .header("x-tag", "a")
        .header("x-tag", "b")
        .build()?;

    session.get(server.url("/both")).await?;
    session.get(server.url("/one")).header("x-tag", "c").await?;

    let seen = server.requests().await;
    assert_eq!(seen[0].header_values("x-tag"), ["a", "b"]);
    assert_eq!(seen[1].header_values("x-tag"), ["c"]);
    Ok(())
}

#[test]
fn a_proxy_list_rejects_a_closure_entry_and_a_session_names_its_proxy_setup() -> Check {
    let closure = || leyline::Proxy::custom(|_| None);
    let nested = Session::builder()
        .proxy(leyline::Proxy::list([closure()]))
        .build();
    let pooled = Session::builder()
        .proxy(leyline::Proxy::list([
            "http://user:secret@127.0.0.1:1",
            "http://127.0.0.1:2",
        ]))
        .build()?;
    let custom = Session::builder().proxy(closure()).build()?;

    assert_eq!(nested.err().map(|error| error.kind()), Some(Kind::Config));
    assert!(pooled.to_string().contains("proxy=pool(2)"), "{pooled}");
    assert!(!format!("{pooled} {pooled:?}").contains("secret"));
    assert!(custom.to_string().contains("proxy=custom"), "{custom}");
    assert_eq!((pooled.proxy(), custom.proxy()), (None, None));
    Ok(())
}

#[test]
fn a_proxy_is_unwind_safe() {
    fn unwind_safe<T: std::panic::UnwindSafe + std::panic::RefUnwindSafe>() {}
    unwind_safe::<leyline::Proxy>();
}

#[test]
fn a_jar_takes_url_strings_and_a_built_cookie_covers_subdomains() -> Check {
    let jar = Jar::new();

    jar.set("https://shop.example/", "sid", "1");
    jar.set("not a url", "lost", "1");
    let shared = Cookie::builder("team", "core")
        .domain("example.com")
        .secure(true)
        .build();
    jar.insert("https://www.example.com/", shared);

    assert_eq!(
        jar.get("https://shop.example/", "sid").as_deref(),
        Some("1")
    );
    assert_eq!(jar.get("not a url", "sid"), None);
    assert_eq!(
        jar.get("https://api.example.com/", "team").as_deref(),
        Some("core")
    );
    assert_eq!(jar.get("http://api.example.com/", "team"), None);
    let names: Vec<String> = jar
        .all()
        .iter()
        .map(|cookie| cookie.name().to_owned())
        .collect();
    assert!(!names.contains(&"lost".to_owned()), "{names:?}");
    Ok(())
}

#[tokio::test]
async fn none_removes_a_size_cap_for_one_request_and_turns_the_cache_off() -> Check {
    let server = testing::Server::http(|_| {
        testing::Response::new(200)
            .header("cache-control", "max-age=60")
            .body(vec![b'x'; 4096])
    })
    .await?;
    let session = Session::builder()
        .max_body_size(1024)
        .cache(1 << 20)
        .cache(None)
        .build()?;

    let capped = session.get(server.url("/big")).bytes().await;
    let open = session
        .get(server.url("/big"))
        .max_body_size(None)
        .bytes()
        .await?;

    assert!(capped.err().is_some());
    assert_eq!(open.len(), 4096);
    assert_eq!(server.requests().await.len(), 2);
    Ok(())
}

#[test]
fn a_builder_builds_a_blocking_session() -> Check {
    let session = Session::builder().retry(0).build_blocking()?;
    let server = session.block_on(testing::Server::http(|_| ok("hi")))?;

    let text = session.block_on(session.get(server.url("/")).text())?;

    assert_eq!(text, "hi");
    Ok(())
}

#[tokio::test]
async fn a_status_error_is_retryable_when_the_retry_rule_would_retry_it() -> Check {
    let server = testing::Server::http(|request| {
        let code = request
            .target()
            .trim_start_matches('/')
            .parse()
            .unwrap_or(500);
        testing::Response::new(code)
    })
    .await?;
    let session = Session::new();
    let mut verdicts = Vec::new();

    for code in [404, 429, 500, 502, 503, 504] {
        let path = format!("/{code}");
        let error = session.get(server.url(&path)).error_for_status().await;
        let error = error.err().ok_or("an error status must fail the request")?;
        verdicts.push((error.kind(), error.is_retryable()));
    }

    let retryable: Vec<bool> = verdicts.iter().map(|(_, retry)| *retry).collect();
    assert!(verdicts.iter().all(|(kind, _)| *kind == Kind::Status));
    assert_eq!(retryable, [false, true, false, true, true, true]);
    Ok(())
}
