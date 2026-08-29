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
        http_version: "2",
        headers: &headers,
    };
    let fp = compute_ja4h(&input);
    let parts: Vec<&str> = fp.split('_').collect();
    assert_eq!(parts.len(), 4);
    assert!(parts[0].starts_with("ge20nn")); // GET, HTTP/2, no cookie, no referer
    assert_eq!(parts[2], "000000000000"); // no cookies
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
        http_version: "2",
        headers: &headers,
    };
    let a = section_a(&input);
    assert!(a.starts_with("po20cr")); // POST, HTTP/2, cookie, referer
    assert!(a.ends_with("enUS"));
}
