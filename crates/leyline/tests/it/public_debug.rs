use std::fmt::Debug;

fn debug<T: Debug>() {}

#[test]
fn every_public_type_implements_debug() {
    debug::<leyline::SessionBuilder>();
    debug::<leyline::RequestBuilder>();
    debug::<leyline::trace::Event<'static>>();
    debug::<leyline::profile::Fingerprint>();
    #[cfg(feature = "multipart")]
    {
        debug::<leyline::multipart::Form>();
        debug::<leyline::multipart::Part>();
    }
    #[cfg(feature = "websocket")]
    debug::<leyline::WebSocket>();
}

#[test]
fn tls_debug_shows_ca_lengths_not_bytes() {
    let shown = format!("{:?}", leyline::Tls::new().ca_der(vec![0xab; 300]));
    assert!(shown.contains("ca_der_lengths: [300]"), "{shown}");
    assert!(!shown.contains("171"), "{shown}");
}

#[tokio::test]
async fn tab_and_link_debug_redact_credentials_and_query() {
    let server = leyline::testing::Server::http(|_| {
        leyline::testing::Response::new(200)
            .header(
                "link",
                "<https://alice:hunter2@example.com/next?token=secret99>; rel=\"next\"",
            )
            .body("ok")
    })
    .await
    .expect("test server starts");
    let session = leyline::Session::new();
    let tab = session.tab();
    let url = server.url("/page?api_key=topsecret");
    let resp = tab.open(&url).await.expect("tab opens page");
    let tab_debug = format!("{tab:?}");
    assert!(!tab_debug.contains("topsecret"), "{tab_debug}");

    let links = resp.links();
    assert_eq!(links.len(), 1);
    let link_debug = format!("{:?}", links[0]);
    assert!(!link_debug.contains("hunter2"), "{link_debug}");
    assert!(!link_debug.contains("secret99"), "{link_debug}");
}
