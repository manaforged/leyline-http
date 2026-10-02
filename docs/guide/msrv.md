# Minimum supported Rust version

The minimum supported Rust version (MSRV) is **1.96**, the `rust-version`
field in the workspace `Cargo.toml`. Every crate sets it. The edition is
2024; the forked `leyline-bssl*` crates keep upstream's 2021.

## What the release gate checks

The release gate runs `cargo check` for the workspace on 1.96 and runs the
tests on the toolchain that `rust-toolchain.toml` pins. A toolchain between
them is expected to work but is not part of the gate.

## When the MSRV moves

The MSRV stays at least two releases behind current stable. It moves only
when Leyline needs a compiler feature, never on a schedule or for style.
A bump ships in a minor release with its own line in
[CHANGELOG.md](https://github.com/manaforged/leyline-http/blob/main/CHANGELOG.md);
a patch release never raises the MSRV.

## Next

Read the [API map](../api.md) to see how the public API fits together.
