use crate::Session;

#[tokio::test]
async fn crlf_header_fails_before_io() {
    let err = Session::new()
        .get("https://example.test/")
        .header("x-a", "1\r\nHost: evil")
        .send()
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("header value"),
        "unexpected: {err}"
    );
}

#[tokio::test]
async fn crlf_url_fails_before_io() {
    let err = Session::new()
        .request(http::Method::GET, "https://example.test/\r\nHost: evil")
        .send()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("URL"), "unexpected: {err}");
}

#[test]
fn content_type_infers_the_preset_on_the_builder_path() {
    use crate::profile::Preset;
    let session = Session::chrome();
    let mut json = session
        .post("https://example.test/")
        .header("content-type", "application/json")
        .body("{}");
    json.prepare().unwrap();
    assert_eq!(json.preset, Some(Preset::Xhr));

    let mut form = session
        .post("https://example.test/")
        .header("content-type", "application/x-www-form-urlencoded")
        .body("a=1");
    form.prepare().unwrap();
    assert_eq!(form.preset, Some(Preset::Form));

    let mut get = session
        .get("https://example.test/")
        .header("content-type", "application/json");
    get.prepare().unwrap();
    assert_eq!(get.preset, Some(Preset::Navigate));

    let mut pinned = session
        .post("https://example.test/")
        .preset(Preset::Navigate)
        .header("content-type", "application/json");
    pinned.prepare().unwrap();
    assert_eq!(pinned.preset, Some(Preset::Navigate));
}
