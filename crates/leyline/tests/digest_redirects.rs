#[path = "core_support/raw_server.rs"]
mod raw_server;

use leyline::{DigestAuth, DnsConfig, ProtocolPolicy, Session};
use raw_server::{RawResponse, RawServer};

fn challenge(nonce: &str, extra: &str) -> RawResponse {
    RawResponse::status(401, "Unauthorized").header(
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
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap()
}

#[tokio::test]
async fn digest_skips_a_challenge_it_cannot_answer() {
    let mut server = RawServer::start(vec![
        RawResponse::status(401, "Unauthorized")
            .header(
                "www-authenticate",
                "Digest realm=\"r\", nonce=\"only-int\", qop=\"auth-int\"",
            )
            .header(
                "www-authenticate",
                "Digest realm=\"r\", nonce=\"plain\", qop=\"auth\"",
            ),
        RawResponse::ok(),
    ])
    .await;

    let resp = session()
        .get(server.url("/protected"))
        .digest_auth(DigestAuth::new("u", "p"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    server.next_request().await;
    let retry = server.next_request().await;
    let auth = retry.header_values("authorization").join("");
    assert!(auth.contains("nonce=\"plain\""), "{auth}");
    server.finish().await;
}

#[tokio::test]
async fn digest_authorizes_a_same_origin_redirect_without_a_new_challenge() {
    let mut server = RawServer::start(vec![
        challenge("same-origin-step", ""),
        RawResponse::redirect("/next"),
        RawResponse::ok(),
    ])
    .await;

    let resp = session()
        .get(server.url("/protected"))
        .digest_auth(DigestAuth::new("u", "p"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    server.next_request().await;
    let answered = server.next_request().await;
    let first = answered.header_values("authorization").join("");
    let redirected = server.next_request().await;
    assert!(redirected.request_line.starts_with("GET /next "));
    let auth = redirected.header_values("authorization").join("");
    assert!(auth.contains("uri=\"/next\""), "{auth}");
    assert!(auth.contains("nonce=\"same-origin-step\""), "{auth}");
    assert_eq!(nc(&auth), nc(&first) + 1, "{first} then {auth}");
    server.finish().await;
}

#[tokio::test]
async fn digest_sends_no_credentials_outside_the_protection_space() {
    let mut server = RawServer::start(vec![
        challenge("protection-space", ", domain=\"/api/\""),
        RawResponse::redirect("/admin/y"),
        RawResponse::ok(),
    ])
    .await;

    let resp = session()
        .get(server.url("/api/x"))
        .digest_auth(DigestAuth::new("u", "p"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    server.next_request().await;
    server.next_request().await;
    let outside = server.next_request().await;
    assert!(outside.request_line.starts_with("GET /admin/y "));
    assert_eq!(
        outside.header_count("authorization"),
        0,
        "{}",
        outside.text()
    );
    server.finish().await;
}

#[tokio::test]
async fn digest_sends_no_credentials_after_a_cross_origin_bounce() {
    let listener = RawServer::bind().await;
    let port = listener.local_addr().unwrap().port();
    let mut server = RawServer::serve(
        listener,
        vec![
            challenge("bounce", ""),
            RawResponse::redirect(format!("http://b.test:{port}/hop")),
            RawResponse::redirect(format!("http://a.test:{port}/back")),
            RawResponse::ok(),
        ],
    );
    let local = "127.0.0.1:0".parse().unwrap();
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .dns(
            DnsConfig::new()
                .resolve_host("a.test", [local])
                .resolve_host("b.test", [local]),
        )
        .build()
        .unwrap();

    let resp = session
        .get(format!("http://a.test:{port}/start"))
        .digest_auth(DigestAuth::new("u", "p"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    for _ in 0..3 {
        server.next_request().await;
    }
    let back = server.next_request().await;
    assert!(back.request_line.starts_with("GET /back "));
    assert_eq!(back.header_count("authorization"), 0, "{}", back.text());
    server.finish().await;
}
