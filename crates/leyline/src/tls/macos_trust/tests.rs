use super::evaluate;

#[test]
fn malformed_certificate_is_not_trusted() {
    assert!(evaluate(&[vec![0x30, 0x00]], "localhost").is_err());
}
