# Contributing to Leyline

Thanks for your interest. Leyline is pre-1.0 and the public API is still
moving — file an issue before you start anything non-trivial so we can agree
on the shape before you write code.

## Dev setup

```bash
git clone https://github.com/manaforged/leyline-http
cd leyline
cargo test --workspace
```

MSRV is `1.85`. The vendored BoringSSL fork lives in `vendor/leyline-ssl*` and
builds from source the first time you compile — expect the initial build to
take a few minutes.

## Before you open a PR

```bash
cargo test --workspace                                    # pre-commit hook runs this
cargo test -p leyline --test tls_peet -- --ignored        # live fingerprint tests
cargo deny --all-features check                           # supply chain gate
cargo fmt --all
cargo clippy --workspace --all-targets
```

The pre-commit hook runs `cargo test --workspace` automatically. If it fails,
fix the cause — do not bypass the hook.

## The claim guard

`crates/leyline/tests/claim_guard.rs` scans the README and every public
`lib.rs` for marketing superlatives (`best`, `undetectable`,
`indistinguishable`, `perfect`, etc.) and fails the build if it finds them.
If a claim cannot be proved by a test in `TESTING.md`, it does not belong in
this repo. When in doubt, phrase things as *what Leyline does* rather than
*how it compares*.

## Adding a new browser profile

1. Copy an existing TOML:
   `cp profiles/chrome/147.toml profiles/chrome/148.toml`
2. Edit `version`, `user_agent`, `sec_ch_ua`, and any TLS / H2 settings that
   moved in the new browser.
3. Add the variant to the `Browser` enum and the registry in
   `crates/profile/src/registry.rs`.
4. Fill in the expected JA4 / H2 fingerprint in the new TOML — the profile
   integrity tests refuse to ship a profile without one.
5. Add a live matcher test in `crates/leyline/tests/tls_peet.rs` following
   the `live_ja4_exact_match_*` pattern.

The `PROFILE_COUNT` constant and the profile-integrity tests enforce that
every enum variant has a TOML file, every TOML file is loaded, and every
profile carries a fingerprint expectation.

## Commit shape

Targeted commits. A refactor, a bug fix, a new
profile, and a docs pass are four commits, not one. The pre-commit hook
catches compile failures — a hook that fails means the commit did not happen
and `--amend` is the wrong tool. Fix the cause, re-stage, create a new commit.
