# Contributing to Leyline

Thanks for your interest. Leyline is pre-1.0 and the public API is still
moving — file an issue before you start anything non-trivial so we can agree
on the shape before you write code.

## Dev setup

Linux / macOS / Git Bash:

```bash
git clone https://github.com/manaforged/leyline-http
cd leyline
./scripts/dev-setup.sh
```

Windows PowerShell:

```powershell
git clone https://github.com/manaforged/leyline-http
cd leyline
.\scripts\dev-setup.ps1
```

MSRV is `1.85`. Leyline vendors its BoringSSL stack in-repo as `leyline-bssl` /
`leyline-bssl-sys` / `leyline-bssl-tokio`. `leyline-bssl-sys` ships prebuilt
BoringSSL for the tier-1 targets, so developer (and git-consumer) builds link it
directly — no source build. Upstream credit and license accounting is in
[`NOTICE`](NOTICE).

The setup script is intentionally smaller than the release gate: it checks
toolchain prerequisites, runs `cargo check -p leyline --all-features`, and
then runs two offline smoke tests. After it passes, the normal local gate is:

```bash
./scripts/verify.sh
```

PowerShell:

```powershell
.\scripts\verify.ps1 -Quick
```

Windows note: use an MSVC Rust toolchain (`stable-x86_64-pc-windows-msvc`),
Visual Studio Build Tools with the C++ workload, CMake, and Strawberry Perl.
The in-repo `crates/leyline-bssl-sys` crate carries prebuilt MSVC BoringSSL
libraries for this target, so a normal Windows developer should not wait on a
BoringSSL source build.

## Syncing the TLS stack

The BoringSSL-facing Rust bindings live in the in-repo `leyline-bssl-sys` crate,
which pins an exact BoringSSL revision (the `crates/leyline-bssl-sys/deps/boringssl`
submodule) plus the patches in `crates/leyline-bssl-sys/patches/`. To pull a
BoringSSL update:

1. Move the submodule to the new revision and rebase the patches
   (`git am --3way`); see `crates/leyline-bssl-sys/patches/SERIES`.
2. Run `scripts/package-bssl.sh` on a host of each target to rebuild the
   prebuilt libraries and bindings.
3. Run `cargo test --workspace --exclude leyline-quiche` and
   `cargo test -p leyline --test tls_peet -- --ignored`. Fingerprint
   tests will catch any regression in cipher order, extension order, or
   GREASE wiring introduced by the bump.

When upstream BoringSSL publishes a CVE, check whether the pinned revision
includes the fix, then bump the submodule + rebuild.

## Before you open a PR

```bash
./scripts/verify.sh            # everything below, in one shot
./scripts/verify.sh           # fast push sanity
./scripts/verify.sh --full    # deliberate release validation
```

The verify script runs:

- `cargo fmt --all --check`
- `cargo clippy --workspace --exclude leyline-quiche --all-targets --no-deps -- -D warnings`
- `cargo doc --workspace --exclude leyline-quiche --no-deps` with `RUSTDOCFLAGS=-D warnings`
- `cargo test --workspace --exclude leyline-quiche`
- `cargo test -p leyline --test tls_peet -- --ignored` (live peet.ws)
- `cargo test -p leyline --test smoke -- --ignored --nocapture`
- `cargo deny --all-features check`
- `cd benches && cargo bench --no-run` when `benches/` is present

`scripts/verify.sh` is the canonical pre-PR and pre-tag gate. Scheduled live
fingerprint verification runs directly on the approved self-hosted machine.

The pre-commit hook runs the offline Leyline workspace suite automatically. If
it fails, fix the cause - do not bypass the hook.

## The claim guard

`crates/leyline/tests/claim_guard.rs` scans the README and every public
`lib.rs` for marketing superlatives (`best`, `undetectable`,
`indistinguishable`, `perfect`, etc.) and fails the build if it finds them.
If a claim cannot be proved by a test, it does not belong in this repo.

## Adding a new browser profile

1. Copy an existing TOML:
   `cp crates/leyline/profiles/chrome/147.toml crates/leyline/profiles/chrome/148.toml`
2. Edit `version`, `user_agent`, `sec_ch_ua`, and any TLS / H2 settings that
   moved in the new browser.
3. Add the variant to the `Browser` enum and the registry in
   `crates/leyline/src/profile/registry.rs`.
4. Fill in the expected JA4 / H2 fingerprint in the new TOML — the profile
   integrity tests refuse to ship a profile without one.
5. Add a live matcher test in `crates/leyline/tests/tls_peet.rs` following
   the `live_ja4_exact_match_*` pattern.

The `PROFILE_COUNT` constant and the profile-integrity tests enforce that
every enum variant has a TOML file, every TOML file is loaded, and every
profile carries a fingerprint expectation.

Record the browser build in `captured_against` on the new TOML.

## Releasing

A release ships from one `v*` tag: the Rust crate (crates.io), the Node addon
(npm/GitHub Packages), and the Python wheels. **[docs/RELEASING.md](docs/RELEASING.md)
is the source of truth** for the procedure and registry details — follow it, not
a hand-run `cargo publish`.

In brief:

1. Bump `workspace.package.version` in the root `Cargo.toml` and mirror it into
   `package.json` (`version` + the three `optionalDependencies`),
   `wrappers/python/pyproject.toml`, and the READMEs — `scripts/verify.sh`
   enforces this. Add a `CHANGELOG.md` entry.
2. Run `scripts/verify.sh` (the local preflight) and merge to `main`.
3. `git tag -s vX.Y.Z -m "leyline vX.Y.Z" && git push origin vX.Y.Z`. The tag
   does not publish anything. Publish each artifact deliberately from its
   supported target host.

## Commit shape

Targeted commits. A refactor, a bug fix, a new
profile, and a docs pass are four commits, not one. The pre-commit hook
catches compile failures — a hook that fails means the commit did not happen
and `--amend` is the wrong tool. Fix the cause, re-stage, create a new commit.
