use super::{name, value};
use http::Method;

#[test]
fn method_rejects_crlf() {
    assert!(Method::from_bytes(b"GET").is_ok());
    assert!(Method::from_bytes(b"GET\r\nHost: evil").is_err());
    assert!(Method::from_bytes(b"").is_err());
    assert!(Method::from_bytes(b"GET /").is_err());
}

#[test]
fn header_rejects_crlf_injection() {
    assert!(name("x-request-id").is_ok());
    assert!(value("1").is_ok());
    assert!(value("1\r\nHost: evil").is_err());
    assert!(name("x-a\r\nHost").is_err());
    assert!(name("x a").is_err());
    assert!(name("x-a\n").is_err());
    assert!(value("a\nb").is_err());
}
