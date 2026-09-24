# Minimum supported Rust version

The minimum supported Rust version (MSRV) is the `rust-version` field in the
workspace `Cargo.toml`. It is **1.96**.

## When the MSRV moves

The MSRV sits at least two releases behind current stable, so a toolchain a
few months old still builds Leyline.

Leyline bumps the MSRV only when a feature it needs requires a newer
compiler. There is no scheduled bump, and no bump for style or convenience.

An MSRV bump ships in a minor release with its own line in
[CHANGELOG.md](https://github.com/manaforged/leyline-http/blob/main/CHANGELOG.md).
A patch release never raises the MSRV.

## What you can build with

Leyline compiles on the MSRV, 1.96, and is tested on current stable. The
release gate compile-checks 1.96 and runs the test suite on stable. A
toolchain between them is expected to work but is not part of the gate.

To check your toolchain, run:

```sh
rustc --version
```
