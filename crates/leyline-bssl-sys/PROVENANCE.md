# BoringSSL prebuilt provenance

`native/CHECKSUMS` pins the committed `native/` libraries and
`src/bindings/` files. Verify a checkout with `sha256sum -c native/CHECKSUMS`
from this directory. A rebuild from source can differ across hosts and
toolchains; run `scripts/package-bssl.sh --verify` and commit the new
checksums when the artifacts change.

## Source revision

- BoringSSL commit `3a9254f16eda7a4c5d2260039ff23456a0a34de4`
- Upstream anchor: the `boringssl_revision` pinned by Chromium's DEPS at
  tag `150.0.7871.26` (the Chrome 150 TLS stack, verbatim)
- ML-DSA TLS signature algorithms are native at this revision (no patch
  needed)

## Symbol prefixing

Every shipped library is built with `-DBORINGSSL_PREFIX=LEYLINE`, so each
export is `LEYLINE_<name>` and a downstream crate can link `openssl-sys`
or `boring-sys` into the same binary. The generated bindings keep the
plain Rust identifier and carry `#[link_name]` for the prefixed export.
Prefixing is not optional; there is no `prefix-symbols` feature.

`scripts/package-bssl.sh` fails the package if an unprefixed
OpenSSL-style export survives.

Targets regenerated with the prefix:

| Target | Prefixed | Date |
| --- | --- | --- |
| aarch64-apple-darwin | yes | 2026-09-01 (Apple clang, cmake 4, macOS 15 arm64) |
| x86_64-unknown-linux-gnu | yes | ubuntu-24.04 |
| aarch64-unknown-linux-gnu | yes | ubuntu-24.04-arm |
| x86_64-pc-windows-msvc | yes | windows-2025, MSVC |

## Carried patches

Applied by `build/main.rs` `ensure_patches_applied` over a fresh
checkout of the source revision (`git apply --3way --whitespace=fix`):

- `patches/leyline-fingerprint.patch` — see its own SHA-256 in
  `native/CHECKSUMS`:
  - record_size_limit + delegated_credentials extensions (Firefox)
  - ECDHE-ECDSA/RSA 3DES cipher suites (Safari/iOS)
  - FFDHE2048/3072 named groups (Firefox)
  - `include/openssl/prefix_symbols.h` entries for the five new exports,
    so `-DBORINGSSL_PREFIX` renames them too

## Reproduction

Run `scripts/package-bssl.sh` on a host of each target (see the script
header for per-target prereqs; Windows/MSVC cross-builds via cargo-xwin
from any host). The script strips and installs
`native/<target>/lib/*.a|*.lib` plus `src/bindings/<target>.rs`; commit
the output together with an updated `native/CHECKSUMS`.

## How to regenerate

The `prebuilt-rebuild` workflow builds the Linux and Windows targets. It is
`workflow_dispatch` only, so dispatch it deliberately.

1. Dispatch `.github/workflows/prebuilt-rebuild.yml`. Leave `targets` at `all`,
   or pass one triple to rebuild a single target.
2. Download the `bssl-prebuilt-<target>` artifacts from the finished run.
3. Copy each artifact over `native/<target>/lib/` and
   `src/bindings/<target>.rs`.
4. Regenerate `native/CHECKSUMS` from the new files. The `sha256-<target>.txt`
   in each artifact records what the runner produced; compare it against the
   copied files before you trust them.
5. Run `scripts/package-bssl.sh --verify`, then commit the libs, the bindings,
   and `native/CHECKSUMS` together.

## crates.io packaging

The publish include list ships these artifacts to every user; the
resulting `.crate` must stay under crates.io's 10 MB cap. Re-measure after
any rebuild.
