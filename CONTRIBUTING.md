# Contributing

Thanks for your interest in Leyline.

## Before you open a pull request

1. Run the release gate:

   ```sh
   ./scripts/verify.sh --full
   ```

   It runs formatting, workspace Clippy with warnings denied, rustdoc with
   warnings denied, the workspace test suite, `cargo-deny`, package and
   consumer checks, and with `--full` the live fingerprint and smoke
   suites. The vendored `leyline-quiche` crate is excluded from the
   clippy, rustdoc, and test gates; the vendored `leyline-bssl*` crates
   still gate. The gate needs Python 3, Node, the 1.96 toolchain through
   rustup, and `cargo-deny` installed; the live suites need network access.
   Set `LEYLINE_SKIP_LIVE_TESTS=1` to skip the live matrix in the pre-commit hook.

   Tests that need the network are marked `#[ignore]`. Run them with
   `cargo nextest run -p leyline-http --run-ignored all` when your change
   touches a browser profile or the TLS, HTTP/2, or HTTP/3 wire path.

2. Keep the change to one topic. A bug fix, its regression test, and its
   changelog line are one pull request.

3. Add a line under `Unreleased` in `CHANGELOG.md` when the change is
   visible to a user of the crate.

## Commit messages

```
area: what the commit does

Problem: One sentence.
Fix: One sentence.
Impact: One sentence. "None" is valid.
Test: The command you ran and the line that proves it.
```

## Browser profiles

A profile under `crates/leyline/profiles/` describes a real capture. Name
the browser build it was captured from in `captured_against`, and keep the
`ja4` and `akamai` goldens next to the fields that produce them. The offline
conformance test gates every profile that fixes its extension order.

## Style

Rust code is formatted by `rustfmt` with the repository defaults. Tests live
in `*_tests.rs` files or under `tests/`, not inside production modules.
