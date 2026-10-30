use leyline::testing::{self, queue};
use leyline::{Protocol, Session};

fn challenge(nonce: &str, extra: &str) -> testing::Response {
    testing::Response::new(401).close().header(
        "www-authenticate",
        format!("Digest realm=\"r\", nonce=\"{nonce}\", qop=\"auth\"{extra}"),
    )
}

fn nc(auth: &str) -> u32 {
    let hex = auth.split("nc=").nth(1).unwrap().get(..8).unwrap();
    u32::from_str_radix(hex, 16).unwrap()
}

fn session() -> Session {
    Session::builder()
        .protocol(Protocol::Http1)
        .build()
        .unwrap()
}

#[tokio::test]
async fn digest_skips_a_challenge_it_cannot_answer() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(401)
            .close()
            .header(
                "www-authenticate",
                "Digest realm=\"r\", nonce=\"only-int\", qop=\"auth-int\"",
            )
            .header(
                "www-authenticate",
                "Digest realm=\"r\", nonce=\"plain\", qop=\"auth\"",
            ),
        testing::Response::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();

    let resp = session()
        .get(server.url("/protected"))
        .digest_auth("u", "p")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    server.next_request().await.unwrap();
    let retry = server.next_request().await.unwrap();
    let auth = retry.header_values("authorization").join("");
    assert!(auth.contains("nonce=\"plain\""), "{auth}");
    server.shutdown().await;
}

#[tokio::test]
async fn digest_authorizes_a_same_origin_redirect_without_a_new_challenge() {
    let server = testing::Server::http(queue(vec![
        challenge("same-origin-step", ""),
        testing::Response::new(302)
            .close()
            .header("location", "/next"),
        testing::Response::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();

    let resp = session()
        .get(server.url("/protected"))
        .digest_auth("u", "p")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    server.next_request().await.unwrap();
    let answered = server.next_request().await.unwrap();
    let first = answered.header_values("authorization").join("");
    let redirected = server.next_request().await.unwrap();
    assert!(redirected.request_line.starts_with("GET /next "));
    let auth = redirected.header_values("authorization").join("");
    assert!(auth.contains("uri=\"/next\""), "{auth}");
    assert!(auth.contains("nonce=\"same-origin-step\""), "{auth}");
    assert_eq!(nc(&auth), nc(&first) + 1, "{first} then {auth}");
    server.shutdown().await;
}

#[tokio::test]
async fn digest_sends_no_credentials_outside_the_protection_space() {
    let server = testing::Server::http(queue(vec![
        challenge("protection-space", ", domain=\"/api/\""),
        testing::Response::new(302)
            .close()
            .header("location", "/admin/y"),
        testing::Response::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();

    let resp = session()
        .get(server.url("/api/x"))
        .digest_auth("u", "p")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    server.next_request().await.unwrap();
    server.next_request().await.unwrap();
    let outside = server.next_request().await.unwrap();
    assert!(outside.request_line.starts_with("GET /admin/y "));
    assert_eq!(
        outside.header_values("authorization").len(),
        0,
        "{}",
        outside.text()
    );
    server.shutdown().await;
}

#[tokio::test]
async fn digest_sends_no_credentials_after_a_cross_origin_bounce() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = testing::Server::http_on(
        listener,
        queue(vec![
            challenge("bounce", ""),
            testing::Response::new(302)
                .close()
                .header("location", format!("http://b.test:{port}/hop")),
            testing::Response::new(302)
                .close()
                .header("location", format!("http://a.test:{port}/back")),
            testing::Response::new(200).body("ok").close(),
        ]),
    )
    .unwrap();
    let local = "127.0.0.1:0".parse().unwrap();
    let session = Session::builder()
        .protocol(Protocol::Http1)
        .resolve("a.test", [local])
        .resolve("b.test", [local])
        .build()
        .unwrap();

    let resp = session
        .get(format!("http://a.test:{port}/start"))
        .digest_auth("u", "p")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    for _ in 0..3 {
        server.next_request().await.unwrap();
    }
    let back = server.next_request().await.unwrap();
    assert!(back.request_line.starts_with("GET /back "));
    assert_eq!(
        back.header_values("authorization").len(),
        0,
        "{}",
        back.text()
    );
    server.shutdown().await;
}
