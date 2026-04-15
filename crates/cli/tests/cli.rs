//! End-to-end CLI tests.
//!
//! - Offline subcommands (`version`, `profile list`, `profile audit`,
//!   `--help`) are snapshot-tested with `insta` so we catch
//!   accidental formatting regressions.
//! - `assert_cmd` drives the binary for behavioral tests (exit codes,
//!   stderr, stdin piping) that a snapshot wouldn't capture well.
//! - Live tests live in `tests/live.rs` behind `#[ignore]`.

use assert_cmd::Command;
use predicates::prelude::*;

/// Build a `leyline` command with deterministic environment so
/// snapshot output doesn't drift (no color, no weird locale).
fn leyline() -> Command {
    let mut cmd = Command::cargo_bin("leyline").expect("leyline binary built");
    cmd.env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .env("TERM", "dumb")
        .env_remove("RUST_LOG");
    cmd
}

#[test]
fn help_prints_usage_and_subcommands() {
    leyline()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: leyline"))
        .stdout(predicate::str::contains("profile"))
        .stdout(predicate::str::contains("inspect"))
        .stdout(predicate::str::contains("completions"));
}

#[test]
fn version_flag_prints_crate_version() {
    leyline()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn version_subcommand_lists_profiles() {
    let out = leyline()
        .arg("version")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("leyline-cli"), "{text}");
    assert!(text.contains("chrome147"), "{text}");
    assert!(text.contains("firefox148"), "{text}");
    assert!(text.contains("safariios18"), "{text}");
}

#[test]
fn profile_list_snapshot() {
    let out = leyline()
        .arg("profile")
        .arg("list")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    insta::assert_snapshot!(String::from_utf8(out).unwrap());
}

#[test]
fn profile_audit_chrome147_has_stable_ja4() {
    let out = leyline()
        .args(["profile", "audit", "chrome147"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    // The JA4 hash for Chrome 147 is deterministic — if this ever
    // changes, we either bumped a profile or broke audit computation.
    assert!(
        text.contains("t13d1516h2"),
        "expected Chrome 147 JA4 prefix `t13d1516h2` in:\n{text}"
    );
}

#[test]
fn unknown_profile_errors_with_helpful_message() {
    leyline()
        .args(["profile", "audit", "netscape1"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown profile"))
        .stderr(predicate::str::contains("profile list"));
}

#[test]
fn missing_url_without_subcommand_errors_cleanly() {
    leyline()
        .assert()
        .failure()
        .stderr(predicate::str::contains("no URL given"));
}

#[test]
fn header_without_colon_errors() {
    leyline()
        .args(["get", "http://localhost:1/", "-H", "not-a-header"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("header must be"));
}

#[test]
fn completions_zsh_emits_something_plausible() {
    leyline()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_leyline"));
}
