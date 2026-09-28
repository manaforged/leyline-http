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
        &[u("https://www.example.com/")]
    ));
    assert!(!is_cross_site(
        &u("https://shop.example.co.uk/a"),
        &[u("https://www.example.co.uk/")]
    ));
    assert!(is_cross_site(
        &u("https://evil.test/a"),
        &[u("https://www.example.com/")]
    ));
    assert!(is_cross_site(
        &u("https://www.example.com/back"),
        &[u("https://www.example.com/"), u("https://other.test/"),]
    ));
}
