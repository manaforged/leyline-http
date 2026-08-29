use super::redacted_url;

#[test]
fn password_is_redacted_username_is_kept() {
    assert_eq!(
        redacted_url("https://user:secretpw@example.com/x?a=1"),
        "https://user:REDACTED@example.com/x?a=1"
    );
    assert_eq!(
        redacted_url("https://example.com/x"),
        "https://example.com/x"
    );
    assert_eq!(
        redacted_url("https://onlyuser@example.com/"),
        "https://onlyuser@example.com/"
    );
}
