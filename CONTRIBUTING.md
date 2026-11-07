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

If [cargo-nextest](https://nexte.st/docs/running/) is installed, select the cases that cover the change. For example:

```sh
CARGO_TARGET_DIR=target/verify cargo nextest run --workspace --locked \
  --features leyline-http/full,leyline-http/bench-internals \
  --test h1_failures --test h2_stream_state \
  --ignore-default-filter --no-tests=fail --retries 0
```

Change the test targets or add a filter for other contracts. An empty selection fails. This command shares the verification build directory and feature selection. Cold builds and source or dependency changes can require compilation. The script's default sanity pass includes packaging; use the focused command during edits.

## Proofs

`scripts/verify.sh --only kani` runs the [Kani](https://model-checking.github.io/kani/) proofs: HPACK integers round-trip for every value and prefix width, arbitrary bytes never decode past the HPACK limit, HTTP/2 frame headers round-trip and ignore the reserved bit, every server SETTINGS id and value follows RFC 9113 section 6.5.2, every HTTP/2 stream state and event follows RFC 9113 section 5.1, and HTTP/2 flow-control windows never pass 2^31-1. The parser tests in the library unit tests check every input up to a fixed length against models of RFC 9112 status lines, RFC 9110 `Content-Range`, WHATWG server-sent events, and the RFC 9113 rules for DATA, HEADERS and PUSH_PROMISE padding, PRIORITY, RST_STREAM, SETTINGS, PING, GOAWAY, WINDOW_UPDATE and unknown frame types.

## Full qualification

Run `scripts/verify.sh --full` before qualifying a candidate. It includes the transport tests, mixed-failure resource workload, packaging, and feature checks. Focused checks do not replace full qualification. Run mutation testing against the affected functions and their tests when checking test strength, not as a replacement for either command.
