use super::*;

#[test]
fn static_is_zero_copy_and_reads_back() {
    let h = HeaderStr::from_static("content-type");
    assert_eq!(h, "content-type");
    assert_eq!(h.as_str(), "content-type");
    assert!(h.eq_ignore_ascii_case("Content-Type"));
}

#[test]
fn from_string_takes_buffer() {
    let h = HeaderStr::from("text/html".to_string());
    assert_eq!(h.as_bytes(), b"text/html");
}

#[test]
fn rejects_non_utf8() {
    HeaderStr::from_utf8(Bytes::from_static(&[0xff, 0xfe])).expect_err("expected Err");
}

#[test]
fn deref_enables_str_methods() {
    let h = HeaderStr::from_static("a=1; Path=/; Secure");
    assert_eq!(h.find('='), Some(1));
    assert_eq!(&h[..1], "a");
}

#[test]
fn borrow_str_hash_contract_holds() {
    let mut m = std::collections::HashMap::new();
    m.insert(HeaderStr::from("content-type".to_string()), 1);
    assert_eq!(m.get("content-type"), Some(&1));
    assert_eq!(m.get("absent"), None);
}
