# leyline-bssl-sys

Leyline's owned BoringSSL FFI. Replaces the upstream `btls-sys` shim so leyline
controls the **exact BoringSSL revision** (and the patches on top of it) that the
TLS engine links — the fingerprint is an emergent property of that revision.

Two paths, same as the old shim:

- **Dev / release (default):** link the committed prebuilt static libs in
  `native/<target>/lib` + the pregenerated `src/bindings/<target>.rs`. No CMake,
  bindgen, Perl, or Go needed. `build.rs` only emits link directives.
- **CI / source rebuild:** `.github/workflows/build-prebuilt.yml` checks out the
  `deps/boringssl` submodule, applies `patches/*` (per `patches/SERIES`), builds
  with `build/main.rs` (cmake + bindgen), and commits the regenerated libs +
  bindings back here.

## Revision + patches

The pinned BoringSSL commit and patch apply-order live in `patches/SERIES`. The
patches (carried from upstream btls) add the Firefox extensions leyline needs
(`set_record_size_limit`, `set_delegated_credentials`) plus PQ support. Bumping
the revision = move the submodule, rebase the patches (`git am --3way`), rebuild,
re-verify the full profile matrix.

Supported targets: `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`,
`aarch64-apple-darwin`.
