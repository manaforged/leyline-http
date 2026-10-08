use super::{name, value};
use http::Method;

#[test]
fn method_rejects_crlf() {
    Method::from_bytes(b"GET").expect("expected Ok");
    Method::from_bytes(b"GET\r\nHost: evil").expect_err("expected Err");
    Method::from_bytes(b"").expect_err("expected Err");
    Method::from_bytes(b"GET /").expect_err("expected Err");
}

#[test]
fn header_rejects_crlf_injection() {
    name("x-request-id").expect("expected Ok");
    value("1").expect("expected Ok");
    value("1\r\nHost: evil").expect_err("expected Err");
    name("x-a\r\nHost").expect_err("expected Err");
    name("x a").expect_err("expected Err");
    name("x-a\n").expect_err("expected Err");
    value("a\nb").expect_err("expected Err");
}
