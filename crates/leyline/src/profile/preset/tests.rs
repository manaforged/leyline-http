use super::*;

fn ctx(firefox: bool) -> HeaderContext<'static> {
    HeaderContext {
        user_agent: "UA",
        sec_ch_ua: "\"Chromium\";v=\"148\"",
        sec_ch_ua_mobile: "?0",
        sec_ch_ua_platform: "Windows",
        accept_language: "en-US,en;q=0.9",
        origin: "https://x.com",
        referer: "https://x.com/",
        firefox,
    }
}

fn names(h: &[HeaderPair]) -> Vec<String> {
    h.iter().map(|(n, _)| n.to_string()).collect()
}

fn value<'a>(h: &'a [HeaderPair], name: &str) -> Option<&'a str> {
    h.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_ref())
}

#[test]
fn firefox_reshapes_client_hints_accept_priority_and_te() {
    let chrome = Preset::Navigate.build_headers(&ctx(false));
    assert!(names(&chrome).iter().any(|n| n == "sec-ch-ua-mobile"));
    assert!(value(&chrome, "accept").unwrap().contains("image/apng"));
    assert_eq!(value(&chrome, "priority"), None);
    assert_eq!(value(&chrome, "te"), None);

    let ff = Preset::Navigate.build_headers(&ctx(true));
    assert!(
        names(&ff).iter().all(|n| !n.starts_with("sec-ch-ua")),
        "firefox must send no Client Hints: {:?}",
        names(&ff)
    );
    assert_eq!(value(&ff, "accept"), Some(FIREFOX_DOC_ACCEPT));
    assert_eq!(value(&ff, "priority"), Some("u=0, i"));
    assert_eq!(value(&ff, "te"), Some("trailers"));

    let ff_xhr = Preset::SameSite.build_headers(&ctx(true));
    assert!(names(&ff_xhr).iter().all(|n| !n.starts_with("sec-ch-ua")));
    assert_eq!(
        value(&ff_xhr, "accept"),
        Some("application/json, text/plain, */*")
    );
    assert_eq!(value(&ff_xhr, "priority"), Some("u=1, i"));
    assert_eq!(value(&ff_xhr, "te"), Some("trailers"));
}

#[test]
fn chrome_xhr_sends_priority_and_accept_language() {
    let xhr = Preset::Xhr.build_headers(&ctx(false));
    assert_eq!(value(&xhr, "priority"), Some("u=1, i"));
    assert_eq!(value(&xhr, "accept-language"), Some("en-US,en;q=0.9"));
    assert_eq!(xhr.iter().filter(|(n, _)| n == "priority").count(), 1);
    let ff = Preset::Xhr.build_headers(&ctx(true));
    assert_eq!(
        ff.iter().filter(|(n, _)| n == "priority").count(),
        1,
        "firefox reshape must replace, not duplicate, priority"
    );
}
