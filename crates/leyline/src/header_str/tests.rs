use super::*;

#[test]
fn rejects_non_utf8() {
    HeaderStr::from_utf8(Bytes::from_static(&[0xff, 0xfe])).expect_err("expected Err");
}

#[test]
fn borrow_str_hash_contract_holds() {
    let mut m = std::collections::HashMap::new();
    m.insert(HeaderStr::from("content-type".to_string()), 1);
    assert_eq!(m.get("content-type"), Some(&1));
    assert_eq!(m.get("absent"), None);
}
