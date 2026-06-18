# Leyline btls-sys shim

This local `btls-sys` patch removes the default CMake/bindgen/libclang path for Leyline developers on Windows/MSVC, Linux/glibc, and macOS/arm64.

The crate keeps the upstream `btls-sys` package name and version so Cargo can patch both Leyline's direct dependency and the transitive dependency from `btls`. It ships:

- pregenerated BoringSSL bindings for `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, and `aarch64-apple-darwin`
- release-built `crypto.lib` / `ssl.lib` (Windows) and `libcrypto.a` / `libssl.a` (Linux + macOS, debug symbols stripped)
- a tiny build script that only emits link directives

For a new platform, run the upstream source build once on that target, then add a matching `src/bindings/<target>.rs` and `native/<target>/lib` bundle and teach `build.rs` + `src/lib.rs` about the target.

## Downstream consumers MUST repeat the patch

Cargo's `[patch.crates-io]` is **not transitive** — it only applies from the
root workspace that is actually being built. Leyline's own root `Cargo.toml`
patches `btls-sys` to this shim, but that patch does **not** flow through a
`leyline = { path = ... }` (or git/version) dependency into a consuming
workspace. Any workspace that depends on leyline pulls the real crates.io
`btls-sys`, which runs bindgen + CMake and needs libclang/Go/Perl on PATH —
and will fail to build without them (`Unable to find libclang`).

Every workspace that depends on leyline must therefore copy the same patch
into its own root `Cargo.toml`, pointing at this shim:

```toml
[patch.crates-io]
btls-sys = { path = "../leyline/crates/btls-sys" }   # adjust the relative path
```

The shim keeps the upstream `btls-sys` name and version precisely so this one
patch line redirects both the direct dependency and the transitive one pulled
in by `btls`.

## Regenerating Linux artifacts

```sh
# Requires clang, cmake, libclang-dev. Builds via upstream btls-sys 0.5.x.
cargo new --lib /tmp/btls-rebuild && cd /tmp/btls-rebuild
printf '[dependencies]\nbtls-sys = "0.5"\n' >> Cargo.toml
cargo build --release
SRC=$(ls -d target/release/build/btls-sys-*/out)
DEST=$LEYLINE_REPO/crates/btls-sys           # set LEYLINE_REPO before running
cp "$SRC/build/libcrypto.a" "$DEST/native/x86_64-unknown-linux-gnu/lib/"
cp "$SRC/build/libssl.a"    "$DEST/native/x86_64-unknown-linux-gnu/lib/"
strip --strip-debug "$DEST/native/x86_64-unknown-linux-gnu/lib/"lib*.a
cp "$SRC/bindings.rs"       "$DEST/src/bindings/x86_64-unknown-linux-gnu.rs"
```

## Regenerating macOS arm64 artifacts

```sh
# Run on an aarch64-apple-darwin host (Apple Silicon Mac).
# Requires Xcode Command Line Tools (clang + libclang.dylib), Homebrew
# cmake + go. Apple's libc++ replaces libstdc++ at link time so the
# build.rs uses `cargo:rustc-link-lib=c++` for this target.
export PATH=/opt/homebrew/bin:$HOME/.cargo/bin:$PATH
export LIBCLANG_PATH=/Library/Developer/CommandLineTools/usr/lib

cargo new --lib /tmp/btls-rebuild && cd /tmp/btls-rebuild
cat > Cargo.toml <<EOF
[package]
name = "btls-rebuild"
version = "0.1.0"
edition = "2024"

[dependencies]
btls-sys = "0.5"

[lib]
path = "src/lib.rs"
EOF
cargo build --release
SRC=$(ls -d target/release/build/btls-sys-*/out | tail -1)
DEST=$LEYLINE_REPO/crates/btls-sys           # set LEYLINE_REPO before running
strip -S "$SRC/build/libcrypto.a" "$SRC/build/libssl.a"
cp "$SRC/build/libcrypto.a" "$DEST/native/aarch64-apple-darwin/lib/"
cp "$SRC/build/libssl.a"    "$DEST/native/aarch64-apple-darwin/lib/"
cp "$SRC/bindings.rs"       "$DEST/src/bindings/aarch64-apple-darwin.rs"
```
