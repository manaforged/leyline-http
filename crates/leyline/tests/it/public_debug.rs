use std::fmt::Debug;

fn debug<T: Debug>() {}

#[test]
fn every_public_type_implements_debug() {
    debug::<leyline::SessionBuilder>();
    debug::<leyline::RequestBuilder>();
    debug::<leyline::trace::Event<'static>>();
    debug::<leyline::Fingerprint>();
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
