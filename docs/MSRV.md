# Minimum supported Rust version

The minimum supported Rust version (MSRV) is the `rust-version` field in the
workspace `Cargo.toml`. It is **1.88**. Read that field rather than this
number if the two disagree.

## When the MSRV moves

Leyline bumps the MSRV only when a feature it needs requires a newer
compiler. There is no scheduled bump, and no bump for style or convenience.

A bump is a breaking-enough change to get its own minor release and its own
line in [CHANGELOG.md](../CHANGELOG.md). You will never find an MSRV bump in a
patch release.

## What you can build with

Leyline supports every stable Rust release from the last six months. If your
toolchain is newer than the MSRV and less than six months old, Leyline builds.
If it is older, upgrade Rust or pin an older Leyline.

To check your toolchain, run:

```sh
rustc --version
```
