# Minimum supported Rust version

The minimum supported Rust version (MSRV) is the `rust-version` field in the
workspace `Cargo.toml`. It is **1.88**. Read that field rather than this
number if the two disagree.

## When the MSRV moves

Leyline bumps the MSRV only when a feature it needs requires a newer
compiler. There is no scheduled bump, and no bump for style or convenience.

A bump is a breaking-enough change to get its own minor release and its own
line in [CHANGELOG.md](https://github.com/manaforged/leyline-http/blob/main/CHANGELOG.md). You will never find an MSRV bump in a
patch release.

## What you can build with

Leyline compiles on the MSRV, 1.88, and is tested on current stable. The
release gate compile-checks 1.88 and runs the test suite on stable. A
toolchain between them is expected to work but is not part of the gate.

To check your toolchain, run:

```sh
rustc --version
```
