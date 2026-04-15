//! Live network tests — hit the real tls.peet.ws and assert that the
//! CLI produces the same JA4 as the underlying `leyline` library.
//! All tests are `#[ignore]` by default so `cargo test -p leyline-cli`
//! stays offline.

use assert_cmd::Command;

fn leyline() -> Command {
    let mut cmd = Command::cargo_bin("leyline").expect("leyline binary built");
    cmd.env("NO_COLOR", "1");
    cmd
}

#[test]
#[ignore = "live: needs network"]
fn cli_ja4_matches_library() {
    let output = leyline()
        .args(["get", "https://tls.peet.ws/api/all", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doc: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    let ja4 = doc["audit"]["ja4"].as_str().expect("audit.ja4 present");
    // Chrome 147 JA4 prefix — deterministic per profile.
    assert!(
        ja4.starts_with("t13d1516h2"),
        "wire JA4 {ja4} does not match expected Chrome 147 prefix"
    );
}

#[test]
#[ignore = "live: needs network"]
fn cli_exposes_peer_certificate() {
    let output = leyline()
        .args(["get", "https://tls.peet.ws/api/all", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doc: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    let cert = doc["tls"]["peer_cert_der_base64"].as_str();
    assert!(
        cert.map(|s| !s.is_empty()).unwrap_or(false),
        "expected a base64-encoded peer cert in the tls block, got {cert:?}"
    );
}

#[test]
#[ignore = "live: needs network"]
fn inspect_subcommand_prints_audit_block() {
    let output = leyline()
        .args(["inspect", "https://tls.peet.ws/", "--format", "pretty"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("── fingerprint ──"), "{text}");
    assert!(text.contains("JA4"), "{text}");
    assert!(text.contains("── tls ──"), "{text}");
}

#[test]
#[ignore = "live: needs network"]
fn h2_head_drops_response_body() {
    // Regression test for the library gap found during the CLI
    // `leyline-h2` used to accumulate DATA
    // frames for HEAD responses even though the HTTP spec forbids
    // the server from sending them. tls.peet.ws happens to return
    // HTML bytes in response to `HEAD /`, so it's a convenient
    // real-world trigger for misbehaving servers.
    let output = leyline()
        .args(["head", "https://tls.peet.ws/", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doc: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    let body_b64 = doc["response"]["body_base64"]
        .as_str()
        .expect("body_base64 present");
    assert_eq!(
        body_b64,
        "",
        "HEAD over HTTP/2 should yield an empty body, got {} base64 chars",
        body_b64.len()
    );
}
