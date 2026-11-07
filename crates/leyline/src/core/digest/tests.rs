use super::*;

#[test]
fn parses_md5_challenge() {
    let hdr = r#"Digest realm="test@example.com", nonce="abc123", qop="auth", algorithm=MD5"#;
    let c = parse_challenge(hdr).unwrap();
    assert_eq!(c.realm, "test@example.com");
    assert_eq!(c.nonce, "abc123");
    assert_eq!(c.qop.as_deref(), Some("auth"));
    assert_eq!(c.algorithm, Algorithm::Md5);
}

fn rfc7616_section_3_9_1_header(algorithm: Algorithm) -> String {
    let challenge = Challenge {
        realm: "http-auth@example.org".into(),
        nonce: "7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v".into(),
        qop: Some("auth, auth-int".into()),
        algorithm,
        opaque: Some("FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS".into()),
        stale: false,
        domain: Vec::new(),
    };
    challenge
        .build_auth_header(
            &DigestAuth::new("Mufasa", "Circle of Life"),
            "GET",
            "/dir/index.html",
            1,
            "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ",
        )
        .expect("auth qop is supported")
}

#[test]
fn rfc7616_md5_vector() {
    let header = rfc7616_section_3_9_1_header(Algorithm::Md5);
    assert!(header.starts_with("Digest username=\"Mufasa\""), "{header}");
    assert!(header.contains("algorithm=MD5"), "{header}");
    assert!(header.contains("qop=auth"), "{header}");
    assert!(header.contains("nc=00000001"), "{header}");
    assert!(
        header.contains("response=\"8ca523f5e9506fed4657c9700eebdbec\""),
        "{header}"
    );
}

#[test]
fn rfc7616_sha256_vector() {
    let header = rfc7616_section_3_9_1_header(Algorithm::Sha256);
    assert!(header.contains("algorithm=SHA-256"), "{header}");
    assert!(
        header.contains(
            "response=\"753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1\""
        ),
        "{header}"
    );
}

#[test]
fn refuses_auth_int_only_challenge() {
    let challenge = Challenge {
        realm: "r".into(),
        nonce: "n".into(),
        qop: Some("auth-int".into()),
        algorithm: Algorithm::Md5,
        opaque: None,
        stale: false,
        domain: Vec::new(),
    };
    let auth = DigestAuth::new("u", "p");
    assert!(
        challenge
            .build_auth_header(&auth, "GET", "/x", 1, "cn")
            .is_none(),
        "auth-int-only challenge must be refused, not answered with a fake auth HA2"
    );
}

#[test]
fn quoted_values_escape_backslash_and_quote() {
    let challenge = Challenge {
        realm: "r".into(),
        nonce: "n".into(),
        qop: Some("auth".into()),
        algorithm: Algorithm::Md5,
        opaque: Some("o\"p".into()),
        stale: false,
        domain: Vec::new(),
    };
    let auth = DigestAuth::new("fo\"o", "p");
    let header = challenge
        .build_auth_header(&auth, "GET", "/x", 1, "cn\\")
        .expect("auth qop");
    assert!(header.contains("username=\"fo\\\"o\""), "{header}");
    assert!(header.contains("opaque=\"o\\\"p\""), "{header}");
    assert!(header.contains("cnonce=\"cn\\\\\""), "{header}");
}

#[test]
fn nonce_count_increments_per_nonce() {
    let _serial = reset_nonce_cache_for_test();
    let n1 = next_nc_for_nonce("nonce-A");
    let n2 = next_nc_for_nonce("nonce-A");
    let n3 = next_nc_for_nonce("nonce-B");
    assert!(n2 > n1, "monotonic within the same nonce");
    assert_eq!(n3, 1, "fresh nonce starts at 1");
}

#[test]
fn nonce_cache_lru_evicts_beyond_cap() {
    let _serial = reset_nonce_cache_for_test();
    for i in 0..DIGEST_NONCE_CACHE_CAP + 16 {
        let nonce = format!("lru-test-{i}");
        let n = next_nc_for_nonce(&nonce);
        assert_eq!(n, 1);
    }
    let restart = next_nc_for_nonce("lru-test-0");
    assert_eq!(
        restart, 1,
        "LRU-evicted nonce restarts at 1; server would see stale=true and re-challenge"
    );
}

#[test]
fn rejects_unknown_algorithm() {
    let hdr = r#"Digest realm="r", nonce="n", algorithm=BLAKE3"#;
    let err = parse_challenge(hdr).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("unsupported algorithm"), "{msg}");
}

#[test]
fn reports_missing_nonce() {
    let hdr = r#"Digest realm="r", qop="auth""#;
    let err = parse_challenge(hdr).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("nonce"), "{msg}");
}

static NONCE_CACHE_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn reset_nonce_cache_for_test() -> std::sync::MutexGuard<'static, ()> {
    let serial = crate::util::lock(&NONCE_CACHE_TESTS);
    crate::util::lock(nonce_cache()).clear();
    serial
}
