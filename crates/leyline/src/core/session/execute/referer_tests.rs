use super::referer_for;

#[test]
fn cross_origin_redirect_sends_origin_only() {
    let referer = referer_for(
        Some("https://user:pw@a.example/reset?token=SECRET"),
        "https://b.example",
    );
    assert_eq!(referer, "https://a.example/");
}

#[test]
fn same_origin_redirect_keeps_path_strips_credentials_and_fragment() {
    let referer = referer_for(
        Some("https://a.example/reset?token=SECRET#tok"),
        "https://a.example",
    );
    assert_eq!(referer, "https://a.example/reset?token=SECRET");
}

#[test]
fn first_step_uses_target_origin() {
    assert_eq!(
        referer_for(None, "https://example.com"),
        "https://example.com/"
    );
}
