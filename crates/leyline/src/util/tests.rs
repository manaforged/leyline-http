use super::*;

#[test]
fn base64_encode_works() {
    assert_eq!(base64_encode("user:pass"), "dXNlcjpwYXNz");
    assert_eq!(base64_encode("a"), "YQ==");
    assert_eq!(base64_encode("ab"), "YWI=");
}

#[test]
fn percent_decode_roundtrips_common_cases() {
    assert_eq!(percent_decode("a%3Db"), "a=b");
    assert_eq!(percent_decode("plain"), "plain");
    assert_eq!(percent_decode("s3cr3t%21"), "s3cr3t!");
    assert_eq!(percent_decode("bad%"), "bad%");
}
