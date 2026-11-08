use super::referer_for;

fn url(s: &str) -> url::Url {
    url::Url::parse(s).unwrap()
}

#[test]
fn cross_origin_redirect_sends_origin_only() {
    let referer = referer_for(
        Some("https://user:pw@a.example/reset?token=SECRET"),
        &url("https://b.example/"),
        "https://b.example",
    );
    assert_eq!(referer, "https://a.example/");
}

#[test]
fn same_origin_redirect_keeps_path_strips_credentials_and_fragment() {
    let referer = referer_for(
        Some("https://a.example/reset?token=SECRET#tok"),
        &url("https://a.example/"),
        "https://a.example",
    );
    assert_eq!(referer, "https://a.example/reset?token=SECRET");
}

#[test]
fn first_step_uses_target_origin() {
    assert_eq!(
        referer_for(None, &url("https://example.com/"), "https://example.com"),
        "https://example.com/"
    );
}
