use std::net::TcpListener;

use crate::testing::{Response, Server, queue};
use crate::{Protocol, Proxy, Session};

type Fields<'a> = &'a [(&'a str, &'a str)];

fn saved_origins(session: &Session, port: u16) -> Vec<(String, u64)> {
    let path = std::env::temp_dir().join(format!(
        "leyline-alt-svc-{}-{port}.json",
        std::process::id()
    ));
    session.save(&path).expect("session saves");
    let bytes = std::fs::read(&path).expect("saved file");
    drop(std::fs::remove_file(&path));
    let saved: serde_json::Value = serde_json::from_slice(&bytes).expect("saved JSON");
    saved["state"]["alt_svc"]
        .as_array()
        .expect("alt_svc array")
        .iter()
        .map(|entry| {
            (
                entry["host"].as_str().expect("host").to_owned(),
                entry["port"].as_u64().expect("port"),
            )
        })
        .collect()
}

async fn known_after(exchanges: &[Fields<'_>]) -> bool {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("address").port();
    let authority = format!("h3=\":{port}\"");
    let responses: Vec<Response> = exchanges
        .iter()
        .map(|fields| {
            fields.iter().fold(Response::new(200), |response, field| {
                response.header(field.0, field.1.replace("H3", &authority))
            })
        })
        .collect();
    let server = Server::http_on(listener, queue(responses)).expect("server starts");
    let session = Session::builder()
        .protocol(Protocol::Http1)
        .proxy(Proxy::new().env(false))
        .build()
        .expect("session");
    for _ in exchanges {
        session.get(server.url("/")).send().await.expect("response");
    }
    saved_origins(&session, port) == [("127.0.0.1".to_owned(), u64::from(port))]
}

async fn noted(fields: Fields<'_>) -> bool {
    known_after(&[fields]).await
}

#[tokio::test]
async fn alt_svc_older_than_its_max_age_is_not_cached() {
    assert!(!noted(&[("alt-svc", "H3; ma=60"), ("age", "120")]).await);
}

#[tokio::test]
async fn alt_svc_reads_the_first_age_of_a_list() {
    assert!(!noted(&[("alt-svc", "H3; ma=60"), ("age", "100, 100")]).await);
}

#[tokio::test]
async fn alt_svc_keeps_the_longest_matching_entry() {
    assert!(noted(&[("alt-svc", "H3; ma=0, H3; ma=3600")]).await);
}

#[tokio::test]
async fn alt_svc_parses_each_field_line_alone() {
    assert!(!noted(&[("alt-svc", "H3; ma=86400; x=\""), ("alt-svc", "clear"),]).await);
}

#[tokio::test]
async fn alt_svc_reads_ma_in_any_case() {
    assert!(!noted(&[("alt-svc", "H3; MA=0")]).await);
}

#[tokio::test]
async fn alt_svc_clear_survives_non_ascii_bytes() {
    assert!(known_after(&[&[("alt-svc", "H3; ma=3600")]]).await);
    assert!(
        !known_after(&[
            &[("alt-svc", "H3; ma=3600")],
            &[("alt-svc", "clear, x=\"café\"")],
        ])
        .await
    );
}

#[tokio::test]
async fn alt_svc_ignores_an_entry_with_a_malformed_ma() {
    assert!(!noted(&[("alt-svc", "H3; ma=0, H3; ma=abc")]).await);
}

#[tokio::test]
async fn alt_svc_age_overflow_is_the_largest_age() {
    assert!(
        !noted(&[
            ("alt-svc", "H3; ma=4294967295"),
            ("age", "18446744073709551616"),
        ])
        .await
    );
}
