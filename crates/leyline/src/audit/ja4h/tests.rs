use super::*;

#[test]
fn ja4h_no_cookies() {
    let headers = vec![
        ("user-agent".into(), "Mozilla/5.0".into()),
        ("accept".into(), "text/html".into()),
        ("accept-language".into(), "en-US,en;q=0.9".into()),
    ];
    let input = Ja4hInput {
        method: "GET",
        version: http::Version::HTTP_2,
        headers: &headers,
    };
    let fp = compute_ja4h(&input);
    let parts: Vec<&str> = fp.split('_').collect();
    assert_eq!(parts.len(), 4);
    assert!(parts[0].starts_with("ge20nn"));
    assert_eq!(parts[2], "000000000000");
    assert_eq!(parts[3], "000000000000");
}

#[test]
fn section_a_format() {
    let headers = vec![
        ("user-agent".into(), "Mozilla/5.0".into()),
        ("accept".into(), "*/*".into()),
        ("cookie".into(), "a=1; b=2".into()),
        ("referer".into(), "https://example.com".into()),
        ("accept-language".into(), "en-US,en;q=0.9".into()),
    ];
    let input = Ja4hInput {
        method: "POST",
        version: http::Version::HTTP_2,
        headers: &headers,
    };
    let a = input.section_a();
    assert!(a.starts_with("po20cr"));
    assert!(a.ends_with("enUS"));
}

#[test]
fn ja4h_handles_multibyte_utf8_and_mixed_case_and_multiple_cookies() {
    let headers = vec![
        ("Cookie".into(), "b=2".into()),
        ("COOKIE".into(), "a=1".into()),
        ("Referer".into(), "https://example.com".into()),
        ("Accept-Language".into(), "日本語".into()),
    ];
    let input = Ja4hInput {
        method: "日本語",
        version: http::Version::HTTP_11,
        headers: &headers,
    };
    let fp = compute_ja4h(&input);
    let parts: Vec<&str> = fp.split('_').collect();
    assert_eq!(parts.len(), 4);
    assert!(parts[0].starts_with("日本11cr01日本語0"));

    let single_headers = vec![
        ("cookie".into(), "a=1; b=2".into()),
        ("referer".into(), "https://example.com".into()),
        ("accept-language".into(), "日本語".into()),
    ];
    let single = Ja4hInput {
        method: "日本語",
        version: http::Version::HTTP_11,
        headers: &single_headers,
    };
    let single_fp = compute_ja4h(&single);
    let single_parts: Vec<&str> = single_fp.split('_').collect();
    assert_eq!(parts[2], single_parts[2]);
    assert_eq!(parts[3], single_parts[3]);
}
