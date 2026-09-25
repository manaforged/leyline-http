use super::*;

fn style(firefox: bool) -> HeaderStyle {
    if firefox {
        HeaderStyle::Gecko
    } else {
        HeaderStyle::Chromium
    }
}

fn ctx() -> HeaderContext<'static> {
    HeaderContext {
        user_agent: "UA",
        sec_ch_ua: "\"Chromium\";v=\"148\"",
        sec_ch_ua_mobile: "?0",
        sec_ch_ua_platform: "Windows",
        accept_language: "en-US,en;q=0.9",
        origin: "https://x.com",
        referer: "https://x.com/",
        fetch_site: "same-origin",
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
    let chrome = style(false).build_headers(Some(Preset::Navigate), &ctx());
    assert!(names(&chrome).iter().any(|n| n == "sec-ch-ua-mobile"));
    assert!(value(&chrome, "accept").unwrap().contains("image/apng"));
    assert_eq!(value(&chrome, "priority"), Some("u=0, i"));
    let names_ref = names(&chrome);
    assert_eq!(names_ref.last().map(String::as_str), Some("priority"));
    assert_eq!(value(&chrome, "te"), None);

    let ff = style(true).build_headers(Some(Preset::Navigate), &ctx());
    assert!(
        names(&ff).iter().all(|n| !n.starts_with("sec-ch-ua")),
        "firefox must send no Client Hints: {:?}",
        names(&ff)
    );
    assert_eq!(
        value(&ff, "accept"),
        Some("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
    );
    assert_eq!(value(&ff, "priority"), Some("u=0, i"));
    assert_eq!(value(&ff, "te"), Some("trailers"));

    let ff_xhr = style(true).build_headers(Some(Preset::SameSite), &ctx());
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
    let xhr = style(false).build_headers(Some(Preset::Xhr), &ctx());
    assert_eq!(value(&xhr, "priority"), Some("u=1, i"));
    assert_eq!(value(&xhr, "accept-language"), Some("en-US,en;q=0.9"));
    assert_eq!(xhr.iter().filter(|(n, _)| n == "priority").count(), 1);
    let ff = style(true).build_headers(Some(Preset::Xhr), &ctx());
    assert_eq!(
        ff.iter().filter(|(n, _)| n == "priority").count(),
        1,
        "firefox reshape must replace, not duplicate, priority"
    );
}
