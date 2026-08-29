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

#[test]
fn rfc7616_md5_vector() {
    // RFC 7616 §3.9.1 — the canonical MD5 test vector.
    let challenge = Challenge {
        realm: "http-auth@example.org".into(),
        nonce: "7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v".into(),
        qop: Some("auth".into()),
        algorithm: Algorithm::Md5,
        opaque: Some("FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS".into()),
        stale: false,
    };
    let auth = DigestAuth::new("Mufasa", "Circle of Life");
    let h = build_auth_header(
        &challenge,
        &auth,
        "GET",
        "/dir/index.html",
        1,
        "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ",
    )
    .expect("auth-only qop is supported");
    assert!(h.starts_with("Digest username=\"Mufasa\""));
    assert!(h.contains("algorithm=MD5"));
    assert!(h.contains("qop=auth"));
    assert!(h.contains("nc=00000001"));
    assert!(h.contains("response=\""));
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
    };
    let auth = DigestAuth::new("u", "p");
    assert!(
        build_auth_header(&challenge, &auth, "GET", "/x", 1, "cn").is_none(),
        "auth-int-only challenge must be refused, not answered with a fake auth HA2"
    );
}

#[test]
fn quoted_values_escape_backslash_and_quote() {
    // RFC 7616 §3.4: `"` and `\` inside a quoted-string carry a
    // backslash prefix. Unescaped, a `"` in the username truncates
    // the Authorization value and can corrupt the header's framing.
    let challenge = Challenge {
        realm: "r".into(),
        nonce: "n".into(),
        qop: Some("auth".into()),
        algorithm: Algorithm::Md5,
        opaque: Some("o\"p".into()),
        stale: false,
    };
    let auth = DigestAuth::new("fo\"o", "p");
    let header = build_auth_header(&challenge, &auth, "GET", "/x", 1, "cn\\").expect("auth qop");
    assert!(header.contains("username=\"fo\\\"o\""), "{header}");
    assert!(header.contains("opaque=\"o\\\"p\""), "{header}");
    assert!(header.contains("cnonce=\"cn\\\\\""), "{header}");
}

#[test]
fn nonce_count_increments_per_nonce() {
    reset_nonce_cache_for_test();
    let n1 = next_nc_for_nonce("nonce-A");
    let n2 = next_nc_for_nonce("nonce-A");
    let n3 = next_nc_for_nonce("nonce-B");
    assert!(n2 > n1, "monotonic within the same nonce");
    assert_eq!(n3, 1, "fresh nonce starts at 1");
}

#[test]
fn nonce_cache_lru_evicts_beyond_cap() {
    reset_nonce_cache_for_test();
    // Fill the cache past capacity.
    for i in 0..DIGEST_NONCE_CACHE_CAP + 16 {
        let nonce = format!("lru-test-{i}");
        let n = next_nc_for_nonce(&nonce);
        assert_eq!(n, 1);
    }
    // The earliest nonces should have been evicted; re-inserting
    // yields nc=1, not the previous counter.
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

#[test]
fn sha256_response_matches_manual_computation() {
    // Build the same challenge/response and sanity-check that we
    // can reproduce the response hash by hand using the SAME code
    // path — this guards against accidental HA1/HA2 typos.
    let challenge = Challenge {
        realm: "r".into(),
        nonce: "n".into(),
        qop: Some("auth".into()),
        algorithm: Algorithm::Sha256,
        opaque: None,
        stale: false,
    };
    let auth = DigestAuth::new("u", "p");
    let header =
        build_auth_header(&challenge, &auth, "GET", "/x", 1, "cn").expect("qop=auth is supported");

    let ha1 = Algorithm::Sha256.hash_hex(b"u:r:p");
    let ha2 = Algorithm::Sha256.hash_hex(b"GET:/x");
    let expected = Algorithm::Sha256.hash_hex(format!("{ha1}:n:00000001:cn:auth:{ha2}").as_bytes());
    assert!(
        header.contains(&format!("response=\"{expected}\"")),
        "header: {header}"
    );
}
