use super::*;

#[test]
fn url_encode_simple() {
    assert_eq!(url_encode("hello world"), "hello+world");
    assert_eq!(url_encode("a=b&c=d"), "a%3Db%26c%3Dd");
    assert_eq!(url_encode("safe-string_v2.0"), "safe-string_v2.0");
}

#[test]
fn url_encode_pairs_works() {
    let pairs = url_encode_pairs(&[
        ("user".to_string(), "alice".to_string()),
        ("pass".to_string(), "s3cr3t!".to_string()),
    ]);
    assert_eq!(pairs, "user=alice&pass=s3cr3t%21");
}
