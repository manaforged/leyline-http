use futures_util::StreamExt;

use super::*;

#[test]
fn boundaries_are_distinct_between_forms() {
    let a = Form::new();
    let b = Form::new();
    assert_ne!(a.boundary, b.boundary);
    assert!(a.boundary.starts_with("----LeylineFormBoundary"));
}

#[test]
fn content_type_carries_boundary() {
    let f = Form::new();
    let ct = f.content_type();
    assert!(ct.starts_with("multipart/form-data; boundary=----LeylineFormBoundary"));
    assert!(ct.contains(&f.boundary));
}

async fn first_part_header(form: Form) -> std::io::Result<bytes::Bytes> {
    let body = form.into_stream_body();
    let mut stream = body;
    match stream.next().await {
        Some(Ok(b)) => Ok(b),
        Some(Err(e)) => Err(e),
        None => Err(std::io::Error::other("empty form")),
    }
}

#[tokio::test]
async fn filename_with_crlf_is_rejected() {
    let form = Form::new().part(
        "upload",
        Part::bytes(b"nope".to_vec()).filename("x\"\r\nInjected: yes\r\n\r\n--evil\r\n"),
    );
    let err = first_part_header(form).await.expect_err("must reject");
    let msg = format!("{err}");
    assert!(
        msg.contains("filename") && msg.contains("control"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn name_with_crlf_is_rejected() {
    let form = Form::new().part("field\r\nInjected: yes", Part::text("v"));
    let err = first_part_header(form).await.expect_err("must reject");
    let msg = format!("{err}");
    assert!(
        msg.contains("name") && msg.contains("control"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn filename_with_dquote_is_backslash_escaped() {
    let form = Form::new().part(
        "upload",
        Part::bytes(b"body".to_vec()).filename("evil\"file.txt"),
    );
    let chunk = first_part_header(form).await.expect("accept with escape");
    let s = std::str::from_utf8(&chunk).expect("utf8");
    assert!(s.contains(r#"filename="evil\"file.txt""#), "chunk: {s}");
}
