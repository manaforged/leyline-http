#![cfg(feature = "html")]

use std::time::Duration;

use leyline::testing::{RecordedRequest, TestResponse, TestServer, queue};
use leyline::{Browser, CompressionConfig, RelayBody, Session, html};

const UPLOAD_PAGE: &str = r#"<html><head><base href="/app/"></head><body>
<form id="up" method="post" action="send" enctype="multipart/form-data">
<input name="title" value="hello">
</form></body></html>"#;

fn upload_site(req: &RecordedRequest) -> TestResponse {
    match req.target.as_str() {
        "/pages/upload" => TestResponse::new(200).body(UPLOAD_PAGE),
        _ => TestResponse::new(200),
    }
}

#[tokio::test]
async fn submit_form_follows_enctype_and_base_href() {
    let server = TestServer::http(upload_site).await.unwrap();
    let tab = Session::builder()
        .browser(Browser::default())
        .protocol(leyline::ProtocolPolicy::Http1)
        .build()
        .unwrap()
        .tab();
    let page = tab.open(server.url("/pages/upload")).await.unwrap();
    let form = html::Form::find(&page.text().await.unwrap(), "up").unwrap();
    tab.submit_form(&form).await.unwrap();
    let posted = server.requests().await.pop().unwrap();
    assert_eq!(posted.target, "/app/send");
    let kind = posted.header("content-type").unwrap_or_default();
    assert!(kind.starts_with("multipart/form-data"), "{kind}");
    let body = String::from_utf8_lossy(&posted.body);
    assert!(body.contains("name=\"title\""), "{body}");
}

fn only(document: &str) -> html::Form {
    let mut forms = html::forms(document);
    assert_eq!(forms.len(), 1, "{document}");
    forms.remove(0)
}

#[test]
fn a_control_joins_the_form_its_form_attribute_names() {
    let form = only(r#"<form id="f" action="/x"></form><input form="f" name="t" value="1">"#);
    assert_eq!(form.field("t"), Some("1"));
}

#[test]
fn a_disabled_fieldset_disables_its_controls() {
    let form = only(
        r#"<form><fieldset disabled><input name="a" value="1"></fieldset><input name="b" value="2"></form>"#,
    );
    assert_eq!(form.field("a"), None);
    assert_eq!(form.field("b"), Some("2"));
}

#[test]
fn only_the_last_checked_radio_is_sent() {
    let form = only(
        r#"<form><input type="radio" name="r" value="x" checked><input type="radio" name="r" value="y" checked></form>"#,
    );
    let values: Vec<_> = form.fields().iter().filter(|(n, _)| n == "r").collect();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].1, "y");
}

#[test]
fn set_replaces_every_value_of_a_name() {
    let mut form = only(
        r#"<form><input type="checkbox" name="tag" value="a" checked><input type="checkbox" name="tag" value="b" checked></form>"#,
    );
    form.set("tag", "z");
    let values: Vec<_> = form.fields().iter().filter(|(n, _)| n == "tag").collect();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].1, "z");
}

#[test]
fn script_text_cannot_open_a_form() {
    let forms = html::forms(
        r#"<script>var s = "</scripts>"; var f = '<form action="evil">';</script><form action="ok"></form>"#,
    );
    assert_eq!(forms.len(), 1);
    assert_eq!(forms[0].action(), "ok");
}

#[test]
fn an_empty_comment_closes_at_once() {
    assert_eq!(html::forms("<!--><form></form><!-- x -->").len(), 1);
}

#[test]
fn named_and_numeric_references_decode_like_a_browser() {
    let form = only(
        r#"<form><input name="a" value="caf&eacute;&hellip;"><input name="b" value="&#128;&#99999999999;"><input name="c" value="x&ampy=1"></form>"#,
    );
    assert_eq!(form.field("a"), Some("café…"));
    assert_eq!(form.field("b"), Some("€\u{FFFD}"));
    assert_eq!(form.field("c"), Some("x&ampy=1"));
}

#[tokio::test]
async fn relay_keeps_content_encoding_on_a_body_that_was_not_decoded() {
    let server = TestServer::http(queue([TestResponse::new(200)
        .close()
        .header("content-encoding", "gzip")
        .body(b"\x1f\x8braw".to_vec())]))
    .await
    .unwrap();
    let resp = Session::builder()
        .compression(CompressionConfig::none())
        .build()
        .unwrap()
        .get(server.url("/"))
        .await
        .unwrap();
    let relayed = resp.relay_headers(RelayBody::Decoded);
    assert_eq!(
        relayed.get("content-encoding").map(|v| v.to_str().unwrap()),
        Some("gzip")
    );
    assert!(resp.bytes().await.unwrap().starts_with(b"\x1f\x8b"));
}

#[test]
fn relay_drops_proxy_authentication_fields() {
    let mut headers = leyline::http::HeaderMap::new();
    headers.insert("proxy-authenticate", "Basic realm=x".parse().unwrap());
    headers.insert("proxy-authorization", "Basic eDp5".parse().unwrap());
    headers.insert("x-kept", "1".parse().unwrap());
    let relayed = leyline::relay_headers(&headers, RelayBody::AsReceived);
    assert!(relayed.get("proxy-authenticate").is_none());
    assert!(relayed.get("proxy-authorization").is_none());
    assert!(relayed.get("x-kept").is_some());
}

#[tokio::test]
async fn pages_does_not_repeat_the_first_page_after_a_redirect() {
    let server = TestServer::http(|req| {
        if req.target == "/items" {
            TestResponse::new(302).close().header("location", "/items/")
        } else {
            TestResponse::new(200)
                .close()
                .header("link", "</items>; rel=\"next\"")
        }
    })
    .await
    .unwrap();
    let mut pages = Session::new().get(server.url("/items")).pages();
    let mut count = 0;
    while let Some(page) = tokio::time::timeout(Duration::from_secs(5), pages.next())
        .await
        .unwrap()
    {
        page.unwrap();
        count += 1;
        assert!(count <= 3, "pages did not stop");
    }
    assert_eq!(count, 1);
}

#[cfg(unix)]
#[tokio::test]
async fn a_download_keeps_the_target_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let server = TestServer::http(queue([TestResponse::new(200).close().body("secret")]))
        .await
        .unwrap();
    let dir = std::env::temp_dir().join(format!("leyline-dlmode-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("token.txt");
    std::fs::write(&target, "old").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    Session::new()
        .get(server.url("/"))
        .download(&target, None)
        .await
        .unwrap();
    let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "secret");
    drop(std::fs::remove_dir_all(&dir));
}

const TEXTAREA_PAGE: &str =
    "<form id=\"t\" method=\"post\" action=\"/send\"><textarea name=\"t\">a\nb</textarea></form>";

#[tokio::test]
async fn a_textarea_line_break_is_sent_as_crlf() {
    let server = TestServer::http(|req| {
        if req.target == "/page" {
            TestResponse::new(200).body(TEXTAREA_PAGE)
        } else {
            TestResponse::new(200)
        }
    })
    .await
    .unwrap();
    let tab = Session::builder()
        .browser(Browser::default())
        .protocol(leyline::ProtocolPolicy::Http1)
        .build()
        .unwrap()
        .tab();
    let page = tab.open(server.url("/page")).await.unwrap();
    let form = html::Form::find(&page.text().await.unwrap(), "t").unwrap();
    tab.submit_form(&form).await.unwrap();
    let posted = server.requests().await.pop().unwrap();
    assert_eq!(posted.body, b"t=a%0D%0Ab");
}

#[test]
fn a_form_attribute_naming_another_element_owns_nothing() {
    let forms = html::forms(
        r#"<div id="x"></div><form id="x" action="/f"></form><input form="x" name="t" value="1">"#,
    );
    assert_eq!(forms[0].field("t"), None);
}

#[test]
fn options_in_a_disabled_optgroup_are_not_sent() {
    let form = only(
        r#"<form><select name="s"><optgroup disabled><option value="a" selected>A</option></optgroup><option value="b">B</option></select></form>"#,
    );
    assert_eq!(form.field("s"), Some("b"));
}

#[test]
fn a_list_box_with_nothing_selected_sends_nothing() {
    let form =
        only(r#"<form><select name="s" size="3"><option value="a">A</option></select></form>"#);
    assert_eq!(form.field("s"), None);
}

#[test]
fn only_a_direct_child_legend_is_exempt_from_a_disabled_fieldset() {
    let form = only(
        r#"<form><fieldset disabled><div><legend><input name="a" value="1"></legend></div></fieldset></form>"#,
    );
    assert_eq!(form.field("a"), None);
}

#[cfg(unix)]
#[tokio::test]
async fn a_download_drops_setuid_bits_from_the_target() {
    use std::os::unix::fs::PermissionsExt;
    let server = TestServer::http(queue([TestResponse::new(200).close().body("bytes")]))
        .await
        .unwrap();
    let dir = std::env::temp_dir().join(format!("leyline-dlsuid-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("tool");
    std::fs::write(&target, "old").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o4755)).unwrap();
    Session::new()
        .get(server.url("/"))
        .download(&target, None)
        .await
        .unwrap();
    let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o755);
    drop(std::fs::remove_dir_all(&dir));
}
