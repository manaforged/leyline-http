use leyline::testing::{TestResponse, TestServer, queue};
use leyline::{Browser, Preset, Session, TlsTrustConfig};

fn browser_only(sent: &leyline::testing::RecordedRequest) -> Vec<String> {
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
    let server = TestServer::http(|_| TestResponse::new(200).close())
        .await
        .unwrap();
    let session = Session::new();
    for preset in [Preset::Navigate, Preset::Xhr, Preset::FormNavigate] {
        session
            .get(server.url("/"))
            .preset(preset)
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
    let server = TestServer::https(queue([TestResponse::new(200)]))
        .await
        .unwrap();
    Session::builder()
        .browser(Browser::default())
        .user_agent("custom-agent/1.0")
        .tls_trust(server.trust())
        .build()
        .unwrap()
        .get(server.url("/"))
        .await
        .unwrap();
    let sent = server.next_request().await.unwrap();
    assert_eq!(sent.header("user-agent"), Some("custom-agent/1.0"));
    assert!(sent.header_values("sec-ch-ua").is_empty());
    assert!(sent.header_values("sec-ch-ua-platform").is_empty());
}

#[tokio::test]
async fn hsts_is_not_learned_without_certificate_checks() {
    let secure = TestServer::https(queue([
        TestResponse::new(200).header("strict-transport-security", "max-age=3600")
    ]))
    .await
    .unwrap();
    let plain = TestServer::http(queue([TestResponse::new(200).close()]))
        .await
        .unwrap();
    let session = Session::builder()
        .tls_trust(TlsTrustConfig::new().danger_accept_invalid_certs(true))
        .build()
        .unwrap();
    session
        .get(format!("https://localhost:{}/", secure.addr().port()))
        .await
        .unwrap();
    let resp = session
        .get(format!("http://localhost:{}/", plain.addr().port()))
        .await
        .unwrap();
    assert_eq!(resp.url().scheme(), "http");
    assert_eq!(resp.status().as_u16(), 200);
}
