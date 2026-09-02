# Contributing

Thanks for your interest in Leyline.

## Before you open a pull request

1. Run the checks the release gate runs:

   ```sh
   cargo fmt --all --check
   cargo clippy -p leyline-http --all-targets
   cargo nextest run -p leyline-http
   ```

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

Rust code is formatted by `rustfmt` with the repository defaults. Public
items carry one-line rustdoc. Tests live in `*_tests.rs` files or under
`tests/`, not inside production modules.
