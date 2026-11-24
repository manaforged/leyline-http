# Contributing

Leyline does not accept pull requests before 1.0. To report a bug or request a feature, open an [issue](https://github.com/manaforged/leyline-http/issues). To report a vulnerability, follow [SECURITY.md](SECURITY.md).

## Build from source

You need rustup, Git, and a C and C++ compiler; on Windows, the MSVC build tools. `scripts/dev-setup.sh` (or `scripts/dev-setup.ps1` on Windows) checks the tools, builds the crate, and runs a short test pass.

```sh
git clone --recurse-submodules https://github.com/manaforged/leyline-http
cd leyline-http
cargo test -p leyline-http --features full,bench-internals
```

## Focused checks

If [cargo-nextest](https://nexte.st/docs/running/) is installed, filter to the cases that cover your change. For example:

```sh
CARGO_TARGET_DIR=target/verify cargo nextest run --workspace --locked \
  --features leyline-http/full,leyline-http/bench-internals \
  --test it -E 'test(h1_failures) | test(h2_stream_state)' \
  --no-tests=fail --retries 0
```

Change the filter expression for other integration modules under `crates/leyline/tests/it/`. An empty selection fails.

## Proofs and sanitizers

- `scripts/verify.sh --only kani` runs all 27 [Kani](https://model-checking.github.io/kani/) symbolic harnesses covering HTTP/1.1 framing, HPACK, HTTP/2 state machines and flow windows, cookie prefixes and expiry, resumable `Content-Range`, and DNS/ECH wire parsing (listed in [QUALIFICATION.md](QUALIFICATION.md#proved)).
- `scripts/verify.sh --only asan` runs the `leyline-bssl` and `leyline-bssl-tokio` FFI tests under AddressSanitizer (with LeakSanitizer on Linux).

## Full qualification

Run `scripts/verify.sh --full` before qualifying a release candidate. It runs formatting, Clippy across 16 feature sets, rustdoc, mdBook, link checks, MSRV 1.96, workspace and subcrate tests, `cargo-deny`, benchmark compilation, and `.crate` packaging with consumer smoke checks. Use `scripts/verify.sh --only mutants` for diff-scoped mutation testing.

Collaborator PRs run verification locally before merge; `ci.yml` runs on external fork PRs and manual workflow dispatches.
