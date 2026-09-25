use super::redact;

#[test]
fn userinfo_is_redacted() {
    assert_eq!(
        redact("https://user:secretpw@example.com/x?a=1"),
        "https://user:***@example.com/x?***"
    );
    assert_eq!(redact("https://example.com/x"), "https://example.com/x");
    assert_eq!(
        redact("https://onlyuser@example.com/"),
        "https://***@example.com/"
    );
}
