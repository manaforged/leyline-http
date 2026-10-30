use leyline::testing::{self, queue};
use leyline::{Browser, RequestKind, Session, Tls};

fn browser_only(sent: &leyline::testing::Request) -> Vec<String> {
    sent.headers
        .iter()
        .map(|(name, _)| name.to_ascii_lowercase())
        .filter(|name| {
            name.starts_with("sec-") || name == "priority" || name == "upgrade-insecure-requests"
        })
        .collect()
}

#[tokio::test]
async fn a_plain_session_sends_no_browser_headers_for_any_preset() {
    let server = testing::Server::http(|_| testing::Response::new(200).close())
        .await
        .unwrap();
    let session = Session::new();
    for preset in [
        RequestKind::Navigate,
        RequestKind::Xhr,
        RequestKind::FormNavigate,
    ] {
        session
            .get(server.url("/"))
            .kind(preset)
            .send()
            .await
            .unwrap();
        let sent = server.next_request().await.unwrap();
        assert!(
            browser_only(&sent).is_empty(),
            "{preset:?}: {:?}",
            browser_only(&sent)
        );
    }
}

#[tokio::test]
async fn a_custom_user_agent_drops_the_client_hints() {
    let server = testing::Server::https(queue([testing::Response::new(200)]))
        .await
        .unwrap();
    Session::builder()
        .browser(Browser::default())
        .headers([("user-agent", "custom-agent/1.0")])
        .tls(server.tls())
        .build()
        .unwrap()
        .get(server.url("/"))
        .send()
        .await
        .unwrap();
    let sent = server.next_request().await.unwrap();
    assert_eq!(sent.header("user-agent"), Some("custom-agent/1.0"));
    assert!(sent.header_values("sec-ch-ua").is_empty());
    assert!(sent.header_values("sec-ch-ua-platform").is_empty());
}

#[tokio::test]
async fn hsts_is_not_learned_without_certificate_checks() {
    let secure = testing::Server::https(queue([
        testing::Response::new(200).header("strict-transport-security", "max-age=3600")
    ]))
    .await
    .unwrap();
    let plain = testing::Server::http(queue([testing::Response::new(200).close()]))
        .await
        .unwrap();
    let session = Session::builder()
        .tls(Tls::new().danger_accept_invalid_certs(true))
        .build()
        .unwrap();
    session
        .get(format!("https://localhost:{}/", secure.addr().port()))
        .send()
        .await
        .unwrap();
    let resp = session
        .get(format!("http://localhost:{}/", plain.addr().port()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.url().scheme(), "http");
    assert_eq!(resp.status().as_u16(), 200);
}

#[tokio::test]
async fn a_header_map_sets_session_and_request_headers() {
    let server = testing::Server::https(queue([testing::Response::new(200)]))
        .await
        .unwrap();
    let mut defaults = http::HeaderMap::new();
    defaults.insert("x-session", http::HeaderValue::from_static("one"));
    let mut extra = http::HeaderMap::new();
    extra.insert("x-request", http::HeaderValue::from_static("two"));
    Session::builder()
        .headers(&defaults)
        .headers([("x-name", "Müller")])
        .tls(server.tls())
        .build()
        .unwrap()
        .get(server.url("/"))
        .headers(&extra)
        .headers([("x-city", "Zürich")])
        .send()
        .await
        .unwrap();
    let sent = server.next_request().await.unwrap();
    assert_eq!(sent.header("x-session"), Some("one"));
    assert_eq!(sent.header("x-request"), Some("two"));
    assert_eq!(sent.header("x-name"), Some("Müller"));
    assert_eq!(sent.header("x-city"), Some("Zürich"));
}
