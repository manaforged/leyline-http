use super::is_cross_site;
use url::Url;

fn u(s: &str) -> Url {
    Url::parse(s).unwrap()
}

#[test]
fn cross_site_navigation_detection() {
    assert!(!is_cross_site(&u("https://example.com/a"), &[]));
    assert!(!is_cross_site(
        &u("https://api.example.com/a"),
        &["https://www.example.com/".to_string()]
    ));
    assert!(!is_cross_site(
        &u("https://shop.example.co.uk/a"),
        &["https://www.example.co.uk/".to_string()]
    ));
    assert!(is_cross_site(
        &u("https://evil.test/a"),
        &["https://www.example.com/".to_string()]
    ));
    assert!(is_cross_site(
        &u("https://www.example.com/back"),
        &[
            "https://www.example.com/".to_string(),
            "https://other.test/".to_string(),
        ]
    ));
}
