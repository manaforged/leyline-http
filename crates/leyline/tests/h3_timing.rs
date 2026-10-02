#![cfg(feature = "http3")]
#[path = "h3_support/mod.rs"]
mod h3_support;
#[path = "tls_support/mod.rs"]
mod tls_support;

use h3_support::{Limits, Reply, h3_server};

#[tokio::test]
async fn http3_responses_report_connect_and_reuse() {
    let server = h3_server(vec![Reply::Body(16), Reply::Body(16)], Limits::default()).await;
    let session = server.session().build().unwrap();
    let first = session.get(server.url()).await.unwrap();
    let timing = first.timing();
    assert!(!timing.reused, "{timing:?}");
    assert!(timing.connect_ms.is_some(), "{timing:?}");
    let second = session.get(server.url()).await.unwrap();
    assert!(second.timing().reused, "{:?}", second.timing());
    assert_eq!(second.timing().connect_ms, None);
}
