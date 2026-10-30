#![cfg(feature = "http3")]
use crate::h3_support;

use h3_support::{Limits, Reply, h3_server};

#[tokio::test]
async fn http3_responses_report_connect_and_reuse() {
    let server = h3_server(vec![Reply::Body(16), Reply::Body(16)], Limits::default()).await;
    let session = server.session().build().unwrap();
    let first = session.get(server.url()).send().await.unwrap();
    assert!(!first.reused());
    assert!(first.connect_time().is_some());
    let second = session.get(server.url()).send().await.unwrap();
    assert!(second.reused());
    assert_eq!(second.connect_time(), None);
}
