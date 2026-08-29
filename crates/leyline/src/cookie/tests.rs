use super::is_cross_site;
use url::Url;

fn u(s: &str) -> Url {
    Url::parse(s).unwrap()
}

#[test]
fn cross_site_navigation_detection() {
    // First request of a chain is always same-site.
    assert!(!is_cross_site(&u("https://example.com/a"), &[]));
    // Subdomain redirect within the same registrable domain — same-site.
    assert!(!is_cross_site(
        &u("https://api.example.com/a"),
        &["https://www.example.com/".to_string()]
    ));
    // co.uk: same registrable domain (example.co.uk) across subdomains.
    assert!(!is_cross_site(
        &u("https://shop.example.co.uk/a"),
        &["https://www.example.co.uk/".to_string()]
    ));
    // Different registrable domain — cross-site.
    assert!(is_cross_site(
        &u("https://evil.test/a"),
        &["https://www.example.com/".to_string()]
    ));
    // Returning to the original site after a cross-site hop stays cross-site.
    assert!(is_cross_site(
        &u("https://www.example.com/back"),
        &[
            "https://www.example.com/".to_string(),
            "https://other.test/".to_string(),
        ]
    ));
}
