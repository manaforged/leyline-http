# Leyline btls-sys shim

This `btls-sys` shim replaces the CMake/bindgen/libclang build with prebuilt artifacts.

The crate keeps the upstream `btls-sys` package name and version so Cargo can patch both Leyline's direct dependency and the transitive dependency from `btls`. It ships:

- pregenerated BoringSSL bindings for `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu`
- release-built `crypto.lib` / `ssl.lib` (Windows) and `libcrypto.a` / `libssl.a` (Linux, debug symbols stripped)
- a tiny build script that only emits link directives

For a new platform, run the upstream source build once on that target, then add a matching `src/bindings/<target>.rs` and `native/<target>/lib` bundle and teach `build.rs` + `src/lib.rs` about the target.

## Regenerating Linux artifacts

```sh
# Requires clang, cmake, libclang-dev. Builds via upstream btls-sys 0.5.x.
cargo new --lib /tmp/btls-rebuild && cd /tmp/btls-rebuild
printf '[dependencies]\nbtls-sys = "0.5"\n' >> Cargo.toml
cargo build --release
SRC=$(ls -d target/release/build/btls-sys-*/out)
DEST=path/to/leyline-http/crates/btls-sys
cp "$SRC/build/libcrypto.a" "$DEST/native/x86_64-unknown-linux-gnu/lib/"
cp "$SRC/build/libssl.a"    "$DEST/native/x86_64-unknown-linux-gnu/lib/"
strip --strip-debug "$DEST/native/x86_64-unknown-linux-gnu/lib/"lib*.a
cp "$SRC/bindings.rs"       "$DEST/src/bindings/x86_64-unknown-linux-gnu.rs"
```
