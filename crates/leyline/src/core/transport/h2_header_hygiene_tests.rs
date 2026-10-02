use super::strip_connection_specific_headers;
use crate::profile::preset::HeaderPair;

fn pairs<'a>(rows: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<HeaderPair> {
    rows.into_iter()
        .map(|(n, v)| (n.to_owned().into(), v.to_owned().into()))
        .collect()
}

#[test]
fn connection_specific_headers_are_stripped() {
    let mut headers = pairs([
        ("user-agent", "x"),
        ("connection", "close"),
        ("Keep-Alive", "timeout=5"),
        ("Upgrade", "websocket"),
        ("Transfer-Encoding", "chunked"),
        ("Proxy-Connection", "keep-alive"),
        ("HTTP2-Settings", "value"),
        ("x-keep", "1"),
    ]);
    strip_connection_specific_headers(&mut headers).unwrap();
    let names: Vec<&str> = headers.iter().map(|(n, _)| n.as_ref()).collect();
    assert_eq!(names, ["user-agent", "x-keep"]);
}

#[test]
fn field_names_are_lowercased() {
    let mut headers = vec![
        ("User-Agent".into(), "Agent/1.0".into()),
        ("x-thing".to_owned().into(), "MiXeD".to_owned().into()),
        ("X-Thing".into(), "second".into()),
        ("TE".into(), "Trailers".into()),
    ];
    strip_connection_specific_headers(&mut headers).unwrap();
    assert_eq!(
        headers,
        pairs([
            ("user-agent", "Agent/1.0"),
            ("x-thing", "MiXeD"),
            ("x-thing", "second"),
            ("te", "Trailers"),
        ])
    );
}

#[test]
fn te_is_allowed_only_for_trailers() {
    let mut headers = pairs([("te", "trailers"), ("te", "trailers, deflate")]);
    strip_connection_specific_headers(&mut headers).unwrap();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].1, "trailers");
}

#[test]
fn duplicate_transfer_encoding_is_rejected() {
    let mut headers = pairs([
        ("Transfer-Encoding", "chunked"),
        ("Transfer-Encoding", "gzip"),
    ]);
    let err = strip_connection_specific_headers(&mut headers).unwrap_err();
    assert!(err.to_string().contains("2 Transfer-Encoding"));
}

#[test]
fn transfer_encoding_with_content_length_is_rejected() {
    let mut headers = pairs([("Content-Length", "5"), ("Transfer-Encoding", "chunked")]);
    let err = strip_connection_specific_headers(&mut headers).unwrap_err();
    assert!(err.to_string().contains("framing would be ambiguous"));
}

#[test]
fn duplicate_transfer_encoding_detection_is_case_insensitive() {
    let mut headers = pairs([
        ("transfer-encoding", "chunked"),
        ("Transfer-Encoding", "chunked"),
    ]);
    assert!(strip_connection_specific_headers(&mut headers).is_err());
}

#[test]
fn a_caller_host_header_is_not_sent_next_to_the_authority() {
    let mut headers = pairs([("host", "evil.test"), ("user-agent", "x")]);
    strip_connection_specific_headers(&mut headers).unwrap();
    let names: Vec<&str> = headers.iter().map(|(n, _)| n.as_ref()).collect();
    assert_eq!(names, ["user-agent"]);
}
