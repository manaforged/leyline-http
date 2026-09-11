use super::{Cow, HeaderPair};
use crate::core::headers::reorder as reorder_headers;

fn h(name: &str, value: &str) -> HeaderPair {
    (Cow::Owned(name.to_string()), Cow::Owned(value.to_string()))
}

#[test]
fn firefox_navigate_order_matches_the_live_capture() {
    use crate::profile::Preset;
    use crate::profile::preset::{FIREFOX_HEADER_ORDER, HeaderContext};
    let ctx = HeaderContext {
        user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:153.0) Gecko/20100101 Firefox/153.0",
        sec_ch_ua: "",
        sec_ch_ua_mobile: "?0",
        sec_ch_ua_platform: "Windows",
        accept_language: "en-US,en;q=0.9",
        origin: "https://tls.peet.ws",
        referer: "",
        firefox: true,
    };
    let mut headers = Preset::Navigate.build_headers(&ctx);
    let order: Vec<String> = FIREFOX_HEADER_ORDER
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    reorder_headers(&mut headers, &order);
    let names: Vec<&str> = headers.iter().map(|(n, _)| n.as_ref()).collect();
    assert_eq!(
        names,
        vec![
            "user-agent",
            "accept",
            "accept-language",
            "accept-encoding",
            "upgrade-insecure-requests",
            "sec-fetch-dest",
            "sec-fetch-mode",
            "sec-fetch-site",
            "sec-fetch-user",
            "priority",
            "te",
        ],
        "firefox navigate header order must match the live capture"
    );
}

#[test]
fn moves_named_headers_to_declared_order() {
    let mut headers = vec![
        h("user-agent", "u"),
        h("accept", "a"),
        h("sec-fetch-site", "s"),
        h("accept-language", "al"),
        h("sec-gpc", "1"),
    ];
    let order = vec![
        "accept".into(),
        "sec-gpc".into(),
        "accept-language".into(),
        "sec-fetch-site".into(),
    ];
    reorder_headers(&mut headers, &order);
    let names: Vec<&str> = headers.iter().map(|(n, _)| n.as_ref()).collect();
    assert_eq!(
        names,
        vec![
            "accept",
            "sec-gpc",
            "accept-language",
            "sec-fetch-site",
            "user-agent",
        ]
    );
}

#[test]
fn missing_names_in_order_are_skipped() {
    let mut headers = vec![h("accept", "a"), h("user-agent", "u")];
    let order = vec!["accept".into(), "sec-gpc".into(), "user-agent".into()];
    reorder_headers(&mut headers, &order);
    let names: Vec<&str> = headers.iter().map(|(n, _)| n.as_ref()).collect();
    assert_eq!(names, vec!["accept", "user-agent"]);
}

#[test]
fn case_insensitive_matching() {
    let mut headers = vec![h("User-Agent", "u"), h("Accept", "a")];
    let order = vec!["accept".into(), "user-agent".into()];
    reorder_headers(&mut headers, &order);
    let names: Vec<&str> = headers.iter().map(|(n, _)| n.as_ref()).collect();
    assert_eq!(names, vec!["Accept", "User-Agent"]);
}
