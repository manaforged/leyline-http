# Leyline btls-sys shim

This local `btls-sys` patch removes the default CMake/bindgen/libclang path for Leyline developers on Windows/MSVC.

The crate keeps the upstream `btls-sys` package name and version so Cargo can patch both Leyline's direct dependency and the transitive dependency from `btls`. It ships:

- pregenerated BoringSSL bindings for `x86_64-pc-windows-msvc`
- release-built `crypto.lib` and `ssl.lib`
- a tiny build script that only emits link directives

For a new platform, run the upstream source build once on that target, then add a matching `src/bindings/<target>.rs` and `native/<target>/lib` bundle and teach `build.rs` about the target.
