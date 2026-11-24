use std::sync::Arc;

use super::*;
use crate::error::BodyLimit;
use crate::header_str::HeaderStr;
use crate::wire::{Body, RequestHeaders, ResponseMode};

#[test]
fn proxy_headers_and_request_headers_cannot_collide_in_cache_key() {
    let url = Arc::new(url::Url::parse("https://example.com/resource").unwrap());
    let mut proxy_with_hdr = crate::net::ProxyUrl::parse("http://127.0.0.1:8080").unwrap();
    proxy_with_hdr.extend_headers(&[("X-Tag".into(), "1".into())]);
    let proxy_plain = crate::net::ProxyUrl::parse("http://127.0.0.1:8080").unwrap();

    let req_proxy_hdr = Prepared {
        method: &Method::GET,
        url: &url,
        headers: RequestHeaders::Owned(Vec::new()),
        body: Body::default(),
        proxy: Some(&proxy_with_hdr),
        response: ResponseMode::Buffered,
        body_limit: BodyLimit::session(1024),
        stale_resend: false,
    };
    let req_origin_hdr = Prepared {
        method: &Method::GET,
        url: &url,
        headers: RequestHeaders::Owned(vec![("x-tag".into(), HeaderStr::from("1"))]),
        body: Body::default(),
        proxy: Some(&proxy_plain),
        response: ResponseMode::Buffered,
        body_limit: BodyLimit::session(1024),
        stale_resend: false,
    };

    assert!(Key::of(&req_proxy_hdr) != Key::of(&req_origin_hdr));
}
