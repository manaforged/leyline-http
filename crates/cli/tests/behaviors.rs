//! Behavioral integration tests — each one asserts a contract
//! documented in `leyline --help`:
//!
//! - `-d @file` round-trips a file body to the server
//! - `-d -` round-trips stdin to the server
//! - `-o file` writes the response body to disk
//! - `-o -` writes the response body to stdout
//! - `-k/--insecure` prints a loud stderr warning
//! - 4xx → exit 22, 5xx → exit 23
//! - `-v profile list` (root flag + subcommand) errors cleanly
//!
//! Every test uses the local `support::start` server so nothing
//! touches the network.

use assert_cmd::Command;

mod support;
use support::{body_of, canned_200_text, canned_404, canned_503, start};

fn leyline() -> Command {
    let mut cmd = Command::cargo_bin("leyline").expect("leyline binary built");
    cmd.env("NO_COLOR", "1").env("TERM", "dumb");
    cmd
}

#[test]
fn post_data_file_round_trips_body() {
    let server = start(|req| {
        let body = body_of(req).to_vec();
        canned_200_text(&format!(
            "got {} bytes: {}",
            body.len(),
            String::from_utf8_lossy(&body)
        ))
    });

    let dir = tempdir();
    let path = dir.join("body.txt");
    std::fs::write(&path, b"hello from file").unwrap();

    let spec = format!("@{}", path.display());
    let out = leyline()
        .args(["post", &server.url, "-d", &spec, "--format", "raw"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("got 15 bytes"), "{text}");
    assert!(text.contains("hello from file"), "{text}");
}

#[test]
fn post_data_stdin_round_trips_body() {
    let server = start(|req| {
        let body = body_of(req).to_vec();
        canned_200_text(&format!("stdin-got:{}", String::from_utf8_lossy(&body)))
    });
    let out = leyline()
        .args(["post", &server.url, "-d", "-", "--format", "raw"])
        .write_stdin("from-pipe")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(String::from_utf8(out).unwrap(), "stdin-got:from-pipe");
}

#[test]
fn output_to_file_writes_body_and_exits_success() {
    let server = start(|_req| canned_200_text("written-to-disk"));
    let dir = tempdir();
    let path = dir.join("out.bin");
    leyline()
        .args([
            "get",
            &server.url,
            "-o",
            path.to_str().unwrap(),
            "--format",
            "raw",
        ])
        .assert()
        .success();
    let got = std::fs::read(&path).unwrap();
    assert_eq!(got, b"written-to-disk");
}

#[test]
fn output_dash_writes_body_to_stdout() {
    let server = start(|_req| canned_200_text("dash-means-stdout"));
    let out = leyline()
        .args(["get", &server.url, "-o", "-", "--format", "raw"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(String::from_utf8(out).unwrap(), "dash-means-stdout");
}

#[test]
fn http_404_exits_with_code_22() {
    let server = start(|_req| canned_404());
    leyline()
        .args(["get", &server.url, "--format", "raw"])
        .assert()
        .code(22);
}

#[test]
fn http_503_exits_with_code_23() {
    let server = start(|_req| canned_503());
    leyline()
        .args(["get", &server.url, "--format", "raw"])
        .assert()
        .code(23);
}

#[test]
fn insecure_flag_prints_warning_to_stderr() {
    // We don't actually need a TLS server — the warning fires on
    // session build, before the request is issued. We give a URL
    // that will fail to connect, then just inspect stderr.
    let err_bytes = leyline()
        .args(["get", "http://127.0.0.1:1/", "-k", "--format", "raw"])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let err = String::from_utf8(err_bytes).unwrap();
    assert!(
        err.contains("TLS peer verification disabled"),
        "expected insecure warning, got: {err}"
    );
}

#[test]
fn root_flag_with_subcommand_is_a_usage_error() {
    // `args_conflicts_with_subcommands` turns this into a clean
    // clap error instead of silently dropping the header flag.
    leyline()
        .args(["-H", "X-Foo: bar", "profile", "list"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn audit_only_pretty_skips_headers_and_body() {
    let server = start(|_req| canned_200_text("this-body-should-not-appear"));
    let out = leyline()
        .args(["get", &server.url, "--audit-only", "--format", "pretty"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    // Body is suppressed.
    assert!(
        !text.contains("this-body-should-not-appear"),
        "body leaked into --audit-only output:\n{text}"
    );
    // Status line is suppressed (no `HTTP/1.1 200 OK` in output).
    assert!(
        !text.contains("HTTP/"),
        "status line leaked into --audit-only output:\n{text}"
    );
    // But the fingerprint block is present.
    assert!(text.contains("── fingerprint ──"), "{text}");
    assert!(text.contains("JA4"), "{text}");
}

#[test]
fn inspect_subcommand_shows_wire_block() {
    // Regression test: `inspect` must not force `--audit-only`; before the fix `inspect`
    // forced `--audit-only` on, which then stripped the wire block —
    // breaking the whole "did we send what we think we sent?" story.
    let server = start(|_req| canned_200_text(""));
    let out = leyline()
        .args(["inspect", &server.url, "--format", "pretty"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("── fingerprint ──"), "{text}");
    assert!(text.contains("── wire ──"), "{text}");
}

#[test]
fn verbose_plus_audit_only_keeps_wire_block() {
    // `-v` sets
    // `show_wire = true`, and the old audit-only gating stripped it.
    // After the fix, both flags are orthogonal and wire is kept.
    let server = start(|_req| canned_200_text("body"));
    let out = leyline()
        .args([
            "get",
            &server.url,
            "-v",
            "--audit-only",
            "--format",
            "pretty",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("── fingerprint ──"), "{text}");
    assert!(text.contains("── tls ──"), "{text}");
    assert!(text.contains("── wire ──"), "{text}");
}

fn tempdir() -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "leyline-cli-behavior-{}",
        std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}
