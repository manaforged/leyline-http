use url::Url;

use super::same_origin;

#[test]
fn same_origin_matches_the_url_origin_rule() {
    let urls = [
        "https://a.test/x",
        "https://a.test:443/y",
        "https://A.test/z",
        "https://a.test:8443/",
        "http://a.test/",
        "http://a.test:80/",
        "https://b.a.test/",
        "wss://a.test/",
        "https://127.0.0.1/",
        "https://[::1]/",
        "blob:https://a.test/uuid",
        "blob:https://b.test/uuid",
        "file:///tmp/a",
        "data:text/plain,hi",
    ];
    for a in urls {
        for b in urls {
            let (a, b) = (Url::parse(a).unwrap(), Url::parse(b).unwrap());
            assert_eq!(same_origin(&a, &b), a.origin() == b.origin(), "{a} vs {b}");
        }
    }
}
