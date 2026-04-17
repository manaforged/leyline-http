# Contributing to Leyline

Thanks for your interest. Leyline is pre-1.0 and the public API is still
moving — file an issue before you start anything non-trivial so we can agree
on the shape before you write code.

## Dev setup

```bash
git clone https://github.com/manaforged/leyline-http
cd leyline
cargo test --workspace
```

MSRV is `1.85`. The vendored BoringSSL bindings live in `vendor/leyline-ssl*`
and build from source the first time you compile — expect the initial
build to take a few minutes. The vendored tree is derived from
[`0x676e67/boring2`](https://github.com/0x676e67/boring2); upstream credit
and license accounting is in [`NOTICE`](NOTICE).

## Syncing the vendored TLS stack

The vendored BoringSSL revision is recorded in
`vendor/leyline-ssl-sys/REVISION`. To pull an upstream update:

1. Clone the upstream we fork from (`0x676e67/boring2` for the Rust
   bindings; `google/boringssl` for the C library) and diff against the
   commit pinned in `REVISION`.
2. Copy changed files into `vendor/leyline-ssl/` and
   `vendor/leyline-ssl-sys/deps/boringssl/`. Do not take files you do not
   need — the vendored tree explicitly omits upstream tests.
3. Re-apply `vendor/leyline-ssl-sys/patches/*.patch` on top. All three
   patches are already inlined in the vendored tree; the build script
   does not re-apply them. If you re-sync, reset to a clean upstream
   tree first, then `git apply` each patch in order.
4. Update `REVISION` with the new upstream commit SHAs and the date.
5. Run `cargo test --workspace` and
   `cargo test -p leyline --test tls_peet -- --ignored`. Fingerprint
   tests will catch any regression in cipher order, extension order, or
   GREASE wiring introduced by the resync.

When upstream BoringSSL publishes a CVE, the first question is whether
the pinned commit includes the fix. `REVISION` is the source of truth.

## Before you open a PR

```bash
./scripts/verify.sh            # everything below, in one shot
./scripts/verify.sh --quick    # skip live network tests
```

The verify script runs:

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS=-D warnings`
- `cargo test --workspace` (excluding vendored BoringSSL crates)
- `cargo test -p leyline --test tls_peet -- --ignored` (live peet.ws)
- `cargo run -p leyline --example smoke`
- `cargo deny --all-features check`
- `cd benches && cargo bench --no-run`

The verify script is the CI. Run it before every PR and before tagging a
release.

The pre-commit hook runs `cargo test --workspace` automatically. If it fails,
fix the cause — do not bypass the hook.

## The claim guard

`crates/leyline/tests/claim_guard.rs` scans the README and every public
`lib.rs` for marketing superlatives (`best`, `undetectable`,
`indistinguishable`, `perfect`, etc.) and fails the build if it finds them.
If a claim cannot be proved by a test in `TESTING.md`, it does not belong in
this repo. When in doubt, phrase things as *what Leyline does* rather than
*how it compares*.

## Adding a new browser profile

1. Copy an existing TOML:
   `cp profiles/chrome/147.toml profiles/chrome/148.toml`
2. Edit `version`, `user_agent`, `sec_ch_ua`, and any TLS / H2 settings that
   moved in the new browser.
3. Add the variant to the `Browser` enum and the registry in
   `crates/profile/src/registry.rs`.
4. Fill in the expected JA4 / H2 fingerprint in the new TOML — the profile
   integrity tests refuse to ship a profile without one.
5. Add a live matcher test in `crates/leyline/tests/tls_peet.rs` following
   the `live_ja4_exact_match_*` pattern.

The `PROFILE_COUNT` constant and the profile-integrity tests enforce that
every enum variant has a TOML file, every TOML file is loaded, and every
profile carries a fingerprint expectation.

## Releasing

1. Bump `workspace.package.version` in the root `Cargo.toml` and add an
   entry to `CHANGELOG.md` (`## x.y.z — YYYY-MM-DD`).
2. Run the full preflight:
   ```
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   cargo test -p leyline --test tls_peet -- --ignored
   cargo run -p leyline --example smoke
   cargo deny --all-features check
   ```
3. Tag: `git tag -s vX.Y.Z -m "leyline vX.Y.Z"`; push tag.
4. Build FFI artifacts locally per target (Linux x86_64, macOS arm64,
   Windows x86_64) with `cargo build --release -p leyline-ffi`. Hash
   each artifact (`sha256sum` / `shasum -a 256`) and attach to the
   GitHub release alongside a signed `SHA256SUMS` file. The repo does
   not ship pre-compiled binaries in-tree — release artifacts are the
   only distribution channel for them.
5. `cargo publish` each workspace crate bottom-up (audit / profile /
   cookies / tcp / h2 / tls / quic / pool / core / leyline / cli). The
   FFI crate ships separately under `leyline-ffi`.

## Commit shape

Targeted commits. A refactor, a bug fix, a new
profile, and a docs pass are four commits, not one. The pre-commit hook
catches compile failures — a hook that fails means the commit did not happen
and `--amend` is the wrong tool. Fix the cause, re-stage, create a new commit.
