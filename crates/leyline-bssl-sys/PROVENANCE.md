# BoringSSL prebuilt provenance

Every byte shipped in `native/` and `src/bindings/` is reproducible from
the inputs below. `native/CHECKSUMS` pins the exact artifacts; verify a
checkout with `sha256sum -c native/CHECKSUMS` from this directory, or
rebuild from source and compare with `scripts/package-bssl.sh --verify`.

## Source revision

- BoringSSL commit `3a9254f16eda7a4c5d2260039ff23456a0a34de4`
- Upstream anchor: the `boringssl_revision` pinned by Chromium's DEPS at
  tag `150.0.7871.26` (the Chrome 150 TLS stack, verbatim)
- ML-DSA TLS signature algorithms are native at this revision (no patch
  needed)

## Carried patches

Applied by `build/main.rs` `ensure_patches_applied` over a fresh
checkout of the source revision (`git apply --3way --whitespace=fix`):

- `patches/leyline-fingerprint.patch` — see its own SHA-256 in
  `native/CHECKSUMS`:
  - record_size_limit + delegated_credentials extensions (Firefox)
  - ECDHE-ECDSA/RSA 3DES cipher suites (Safari/iOS)
  - FFDHE2048/3072 named groups (Firefox)

## Reproduction

Run `scripts/package-bssl.sh` on a host of each target (see the script
header for per-target prereqs; Windows/MSVC cross-builds via cargo-xwin
from any host). The script strips and installs
`native/<target>/lib/*.a|*.lib` plus `src/bindings/<target>.rs`; commit
the output together with an updated `native/CHECKSUMS`.

## crates.io packaging

The publish include list ships these artifacts to every user; the
resulting `.crate` must stay under crates.io's 10 MB cap.
