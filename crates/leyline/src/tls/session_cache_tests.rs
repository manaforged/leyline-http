use crate::Session;
use crate::testing::{Response, Server};

#[tokio::test]
async fn the_session_cache_keeps_the_newest_256_tickets() {
    let server = Server::https(|_| Response::new(200).body("ok"))
        .await
        .expect("server");
    let session = || {
        Session::builder()
            .tls(server.tls())
            .build()
            .expect("session")
    };

    let first = session();
    let response = first.get(server.url("/")).send().await.expect("response");
    assert_eq!(response.text().await.expect("body"), "ok");
    let saved = serde_json::to_value(first.connection_state()).expect("state");
    let ticket = &saved["tls_sessions"][0];
    let key = ticket["key"].as_str().expect("the handshake left a ticket");
    let (_, policy) = key.split_once('|').expect("key names its route");

    let tickets: Vec<serde_json::Value> = (0..300)
        .map(|n| {
            serde_json::json!({
                "key": format!("host-{n}:443|{policy}"),
                "session": ticket["session"],
            })
        })
        .collect();
    let state = serde_json::json!({ "tls_sessions": tickets });

    let restored = session();
    restored.restore(&serde_json::from_value(state).expect("state"));

    let kept = serde_json::to_value(restored.connection_state()).expect("state");
    let keys: Vec<&str> = kept["tls_sessions"]
        .as_array()
        .expect("tickets")
        .iter()
        .map(|ticket| ticket["key"].as_str().expect("key"))
        .collect();
    assert_eq!(keys.len(), 256);
    assert!(keys[0].starts_with("host-44:443|"), "{}", keys[0]);
    assert!(keys[255].starts_with("host-299:443|"), "{}", keys[255]);
}
