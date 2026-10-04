# BoringSSL provenance

## Upstream base

`leyline-bssl-sys`, `leyline-bssl`, and `leyline-bssl-tokio` are trimmed forks
of Cloudflare's `boring-sys`, `boring`, and `tokio-boring`:

- Repository: https://github.com/cloudflare/boring
- Tag: `v5.2.0`
- Commit: `6fdd0a54e81a1eecc76e587c685439d3f2ffdd09`

Changes from upstream:

- Crates renamed. `leyline-bssl-sys` declares `links = "leyline_bssl"`.
- Removed features: `fips`, `rpk`, `mlkem`, `mldsa`, `prf`, `credential`,
  `pq-experimental`, `underscore-wildcards`, `relax-cert-validation`,
  `allow-crl-extensions-bad-version`, and `legacy-compat-deprecated`.
- Removed modules that Leyline does not use: `aead`, `aes`, `base64`,
  `ecdsa`, `fips`, `memcmp`, `mldsa`, `mlkem`, `pkcs12`, `pkcs5`, `prf`,
  `rand`, `sha`, `sign`, and `ssl::credential`.
- Removed the upstream BoringSSL patches (`boring-pq`, `rpk`,
  `underscore-wildcards`, `relax-cert-validation`, `bad-cert-verification`)
  and the `BORING_BSSL_INSTALL_DIR` export.
- Added wrappers: `SslContextBuilder::{set_sigalgs, set_record_size_limit,
  set_delegated_credentials, set_extension_order, set_extension_tail,
  set_tls13_cipher_order, set_grease_sigalgs_enabled}`,
  `SslRef::{set_requested_trust_anchors, add_application_settings,
  set_alps_use_new_codepoint, set_tls12_extensions}`,
  `SslConnector::bare_builder`, and `CertificateCompressionAlgorithm::ZSTD`.
- The build sets `BORINGSSL_PREFIX=LEYLINE`, maps build paths, and stops on
  an unsupported target.
- The build variables use the `LEYLINE_BSSL_` prefix where `boring-sys` uses
  `BORING_BSSL_`. The build reads no `BORING_BSSL_*` variable, so a value set
  for `boring-sys` does not reach Leyline.
- Edition 2021 is kept from upstream.

## BoringSSL revision

- Commit `ac39ea6853833c1f18fd23614091d11855e71752`, vendored as the
  `deps/boringssl` submodule.
- It is the `boringssl_revision` in Chromium's DEPS at tag `154.0.8037.58`.
  Cloudflare v5.2.0 pins `e2a57cfb4d915b4ba820585aef9fdee7bca13fe5`, 372
  commits older. Leyline keeps the Chrome revision so that the TLS wire
  output stays the same.

## Carried patches

`build/source.rs` applies every `patches/*.patch` in name order with
`git apply --whitespace=fix`. If `LEYLINE_BSSL_SOURCE_PATH` is set, it applies
them in place to that tree. Otherwise, it applies them to a copy of the source
in `OUT_DIR`. `LEYLINE_BSSL_PATH` and `LEYLINE_BSSL_ASSUME_PATCHED` skip this
step. The patches are:

1. `0001-leyline-fingerprint.patch`
   - `SSL_CTX_set_record_size_limit` and `SSL_set_record_size_limit`
     (RFC 8449).
   - `SSL_CTX_set_delegated_credentials` (RFC 9345).
   - `SSL_CTX_set_extension_order` and `SSL_CTX_set_tls13_cipher_order`.
   - The ECDHE-ECDSA and ECDHE-RSA 3DES cipher suites.
   - The FFDHE2048 and FFDHE3072 groups.
   - Duplicate signature algorithms are allowed in the preference list.
   - `prefix_symbols.h` entries for the new exports.

   These changes first shipped in 0x676e67's `btls` and `boring2` forks
   (https://github.com/0x676e67/btls).
2. `0002-leyline-symbol-prefix.patch`
   - Renames the `ssl_st` and `ssl_session_st` C++ structs to
     `LEYLINE_ssl_st` and `LEYLINE_ssl_session_st`. Their destructors are
     then prefixed too.
3. `0003-leyline-tls12-extensions.patch`
   - `SSL_set_tls12_extensions` keeps `extended_master_secret` and
     `renegotiation_info` in a ClientHello whose minimum version is TLS 1.3.
     Firefox sends both in its QUIC ClientHello.
4. `0004-leyline-extension-tail.patch`
   - `SSL_CTX_set_extension_tail` keeps the listed extensions last, in the
     listed order, when `permute_extensions` shuffles the ClientHello.
     Firefox shuffles its QUIC ClientHello and keeps
     `quic_transport_parameters` and ECH last.
5. `0005-leyline-client-hello-padding.patch`
   - Restores the padding extension (RFC 7685), which pads a cleartext
     ClientHello of 256 to 511 bytes to 512 bytes. BoringSSL removed it in
     commit `eae46df1e6c1df0556ee4106bcab75657eb8181e`. A ClientHello
     without a post-quantum key share, such as Safari 18's, falls in that
     range, and the browser sends the extension.

BoringSSL provides `SSL_CTX_set_grease_sigalgs_enabled` since commit
`29e593e29165df578ab778269a1f04da2055c32f`. It puts one GREASE value
(RFC 8701) first in the ClientHello `signature_algorithms` list, which
Chrome 152 and later send. BoringSSL runs its symbol-prefix audit only in its
test targets, so a prefixed build does not need Go.

## Symbol prefix

Every C export is `LEYLINE_<name>`. BoringSSL's C++ code is in the
`bssl::LEYLINE` namespace, and patch 0002 renames the two global C++
structs. The generated bindings carry `#[link_name]` for the prefixed
exports. The remaining unprefixed globals are C++ standard library and
compiler support symbols (weak or COMDAT), which do not clash.

## Build

The build compiles BoringSSL from source with CMake and uses the bindings
committed under `bindings/` for the target. It needs CMake 3.22 or later, a
C and C++ compiler, and Git. On Windows it also needs the MSVC build tools
and NASM. The `bindgen` feature generates the bindings at build time
instead and needs libclang.

- Build paths: `-ffile-prefix-map` (MSVC: `/d1trimfile`) maps `OUT_DIR` to
  `/build`, and the source tree to `/build/boringssl`.
- macOS: `CMAKE_OSX_DEPLOYMENT_TARGET` comes from
  `MACOSX_DEPLOYMENT_TARGET`, or 11.0.
- MSVC: `+crt-static` selects the static CRT (`MultiThreaded`).

## crates.io packaging

The `.crate` ships the BoringSSL sources that the build needs, not binaries.
Measure its size with `cargo package --no-verify` before a release. The
crates.io limit is 10 MiB.
