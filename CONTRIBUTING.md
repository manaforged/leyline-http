# Contributing

Leyline does not accept pull requests before 1.0. To report a bug or request a feature, open an [issue](https://github.com/manaforged/leyline-http/issues). To report a vulnerability, follow [SECURITY.md](SECURITY.md).

## Build from source

You need rustup, Git, and a C and C++ compiler; on Windows, the MSVC build tools. `scripts/dev-setup.sh` (or `scripts/dev-setup.ps1` on Windows) checks the tools, builds the crate, and runs a short test pass.

```sh
git clone --recurse-submodules https://github.com/manaforged/leyline-http
cd leyline-http
cargo test -p leyline-http --features full
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

## Full qualification

Run `scripts/verify.sh --full` before qualifying a candidate. It includes the transport tests, mixed-failure resource workload, packaging, feature checks, and live checks. Focused checks do not replace full qualification. Run mutation testing against the affected functions and their tests when checking test strength, not as a replacement for either command.
