use super::*;

fn preset_xhr() -> Vec<HeaderPair> {
    vec![
        ("sec-ch-ua".into(), "chrome".into()),
        ("sec-ch-ua-mobile".into(), "?0".into()),
        ("sec-ch-ua-platform".into(), "\"Windows\"".into()),
        ("user-agent".into(), "Mozilla/5.0".into()),
        ("accept".into(), "application/json".into()),
        ("sec-fetch-site".into(), "same-origin".into()),
        ("sec-fetch-mode".into(), "cors".into()),
        ("sec-fetch-dest".into(), "empty".into()),
        ("accept-encoding".into(), "gzip".into()),
        ("accept-language".into(), "en-US".into()),
    ]
}

fn never_sensitive(_: &str) -> bool {
    false
}

#[test]
fn plain_unknown_header_appends_at_end() {
    let mut headers = preset_xhr();
    let mut extra = HeaderList::new();
    extra.set("x-custom", "val");
    apply_extra_headers(&mut headers, &extra, false, &never_sensitive);
    assert_eq!(headers.last().unwrap().0, "x-custom");
}

#[test]
fn plain_known_header_rides_inferred_anchor() {
    // `authorization` has an inferred AfterUserAgent anchor.
    let mut headers = preset_xhr();
    let mut extra = HeaderList::new();
    extra.set("authorization", "Bearer t");
    apply_extra_headers(&mut headers, &extra, false, &never_sensitive);
    let ua = headers.iter().position(|(k, _)| k == "user-agent").unwrap();
    assert_eq!(headers[ua + 1].0, "authorization");
}

#[test]
fn anchored_header_splices_after_anchor() {
    let mut headers = preset_xhr();
    let mut extra = HeaderList::new();
    extra.append_anchored(HeaderAnchor::AfterUserAgent, "x-extra-6", "c");
    apply_extra_headers(&mut headers, &extra, false, &never_sensitive);
    let ua = headers.iter().position(|(k, _)| k == "user-agent").unwrap();
    assert_eq!(headers[ua + 1].0, "x-extra-6");
}

#[test]
fn multiple_anchored_same_anchor_preserve_caller_order() {
    let mut headers = preset_xhr();
    let mut extra = HeaderList::new();
    extra.append_anchored(HeaderAnchor::AfterCchUaMobile, "x-a0", "0");
    extra.append_anchored(HeaderAnchor::AfterCchUaMobile, "x-b", "b");
    extra.append_anchored(HeaderAnchor::AfterCchUaMobile, "x-a", "a");
    apply_extra_headers(&mut headers, &extra, false, &never_sensitive);
    let names: Vec<&str> = headers.iter().map(|(k, _)| k.as_ref()).collect();
    let mobile = names.iter().position(|&n| n == "sec-ch-ua-mobile").unwrap();
    assert_eq!(&names[mobile + 1..mobile + 4], &["x-a0", "x-b", "x-a"]);
}

#[test]
fn preset_owned_name_replaced_in_place() {
    let mut headers = preset_xhr();
    let original_ua_idx = headers.iter().position(|(k, _)| k == "user-agent").unwrap();
    let mut extra = HeaderList::new();
    extra.set("user-agent", "custom-agent");
    apply_extra_headers(&mut headers, &extra, false, &never_sensitive);
    assert_eq!(headers[original_ua_idx].0, "user-agent");
    assert_eq!(headers[original_ua_idx].1, "custom-agent");
}

#[test]
fn anchored_headers_interleave_at_each_anchor() {
    // Regression test for a third-party SDK header pattern
    // — seven headers at five anchors.
    let mut headers = vec![
        ("sec-ch-ua".into(), "chrome".into()),
        ("sec-ch-ua-mobile".into(), "?0".into()),
        ("sec-ch-ua-platform".into(), "\"Windows\"".into()),
        ("user-agent".into(), "Mozilla/5.0".into()),
        ("accept".into(), "*/*".into()),
        ("content-type".into(), "application/json".into()),
        ("origin".into(), "https://api.example.com".into()),
        ("accept-encoding".into(), "gzip".into()),
    ];
    let mut extra = HeaderList::new();
    extra.append_anchored(HeaderAnchor::AfterCchUaPlatform, "x-extra-5", "z");
    extra.append_anchored(HeaderAnchor::AfterCchUa, "x-extra-1", "f");
    extra.append_anchored(HeaderAnchor::AfterCchUaMobile, "x-extra-2", "a0");
    extra.append_anchored(HeaderAnchor::AfterCchUaMobile, "x-extra-3", "b");
    extra.append_anchored(HeaderAnchor::AfterCchUaMobile, "x-extra-4", "a");
    extra.append_anchored(HeaderAnchor::AfterUserAgent, "x-extra-6", "c");
    extra.append_anchored(HeaderAnchor::AfterContentType, "x-extra-7", "d");
    apply_extra_headers(&mut headers, &extra, false, &never_sensitive);
    let names: Vec<&str> = headers.iter().map(|(k, _)| k.as_ref()).collect();
    assert_eq!(
        names,
        vec![
            "sec-ch-ua",
            "x-extra-1",
            "sec-ch-ua-mobile",
            "x-extra-2",
            "x-extra-3",
            "x-extra-4",
            "sec-ch-ua-platform",
            "x-extra-5",
            "user-agent",
            "x-extra-6",
            "accept",
            "content-type",
            "x-extra-7",
            "origin",
            "accept-encoding",
        ]
    );
}

#[test]
fn before_anchor_inserts_before_target() {
    let mut headers = preset_xhr();
    let mut extra = HeaderList::new();
    extra.append_anchored(HeaderAnchor::BeforeAcceptEncoding, "x-last-chance", "v");
    apply_extra_headers(&mut headers, &extra, false, &never_sensitive);
    let ae_idx = headers
        .iter()
        .position(|(k, _)| k == "accept-encoding")
        .unwrap();
    assert_eq!(headers[ae_idx - 1].0, "x-last-chance");
}

#[test]
fn sensitive_stripped_on_cross_origin_redirect() {
    let mut headers = preset_xhr();
    let mut extra = HeaderList::new();
    extra.set("authorization", "Bearer t");
    extra.set("x-benign", "ok");
    let sensitive = |name: &str| name.eq_ignore_ascii_case("authorization");
    apply_extra_headers(&mut headers, &extra, true, &sensitive);
    assert!(!headers.iter().any(|(k, _)| k == "authorization"));
    assert!(headers.iter().any(|(k, _)| k == "x-benign"));
}

#[test]
fn anchor_absent_falls_back_to_end() {
    // Navigate preset has no content-type; AfterContentType
    // anchor falls back to appending at the end.
    let mut headers = vec![
        ("sec-ch-ua".into(), "chrome".into()),
        ("user-agent".into(), "Mozilla/5.0".into()),
        ("accept".into(), "text/html".into()),
        ("accept-encoding".into(), "gzip".into()),
    ];
    let mut extra = HeaderList::new();
    extra.append_anchored(HeaderAnchor::AfterContentType, "x-d", "d");
    apply_extra_headers(&mut headers, &extra, false, &never_sensitive);
    assert_eq!(headers.last().unwrap().0, "x-d");
}
