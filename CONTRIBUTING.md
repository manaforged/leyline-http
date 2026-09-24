# Contributing

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
   still gate. The gate needs Python 3, Node, rustup (the toolchain comes from
   `rust-toolchain.toml`), and `cargo-deny`. The live suites need network
   access.

   Tests that need the network are marked `#[ignore]`. Run them with
   `cargo nextest run -p leyline-http --run-ignored all` when your change
   touches a browser profile or the TLS, HTTP/2, or HTTP/3 wire path.

2. Keep the change to one topic. A bug fix, its regression test, and its
   changelog line are one pull request.

3. Add a line under `Unreleased` in `CHANGELOG.md` when the change is
   visible to a user of the crate.

## Pull request description

Pull requests are squash-merged. Fill in the template: the problem, the fix,
the user-visible impact, and the command you ran to test it. Commit messages
inside the branch can use any clear style.

## Browser profiles

A profile under `crates/leyline/profiles/` describes a real capture. Name
the browser build it was captured from in `captured_against`, and keep the
`ja4` and `akamai` goldens next to the fields that produce them. The offline
conformance test gates every profile that fixes its extension order.
`scripts/profile-oneshot.sh` captures new browser builds and lands their
profiles; run it with `status` to see which versions are missing.

## Style

Rust code is formatted by `rustfmt` with the repository defaults. Tests live
in `*_tests.rs` files or under `tests/`, not inside production modules.
