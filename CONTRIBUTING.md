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

MSRV is `1.85`. Leyline uses `btls` / `btls-sys` for BoringSSL. This repo
patches `btls-sys` to a local Windows/MSVC shim for developer builds; public
crates.io consumers use the upstream source-build path unless they provide
their own patch. Upstream credit and license accounting is in [`NOTICE`](NOTICE).

The setup script is intentionally smaller than the release gate: it checks
toolchain prerequisites, runs `cargo check -p leyline --all-features`, and
then runs two offline smoke tests. After it passes, the normal local gate is:

```bash
./scripts/verify.sh --quick
```

PowerShell:

```powershell
.\scripts\verify.ps1 -Quick
```

Windows note: use an MSVC Rust toolchain (`stable-x86_64-pc-windows-msvc`),
Visual Studio Build Tools with the C++ workload, CMake, and Strawberry Perl.
The in-repo `crates/btls-sys` shim carries prebuilt MSVC BoringSSL libraries
for this target, so a normal Windows developer should not wait on a BoringSSL
source build.

## Syncing the TLS stack

The BoringSSL-facing Rust bindings are provided by `btls` / `btls-sys`. To pull
an upstream update:

1. Bump the `btls`, `btls-sys`, and `tokio-btls` workspace dependency versions
   together.
2. If the local shim still applies, refresh `crates/btls-sys` against the same
   upstream release and regenerate any prebuilt bindings/artifacts it carries.
3. Run `cargo test --workspace --exclude leyline-quiche` and
   `cargo test -p leyline --test tls_peet -- --ignored`. Fingerprint
   tests will catch any regression in cipher order, extension order, or
   GREASE wiring introduced by the resync.

When upstream BoringSSL publishes a CVE, check whether the pinned `btls-sys`
release includes the fix, then update the workspace dependency set together.

## Before you open a PR

```bash
./scripts/verify.sh            # everything below, in one shot
./scripts/verify.sh --quick    # skip live network tests
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

GitHub Actions runs CI: `test.yml` runs version/optionalDependencies parity,
`cargo deny`, and the workspace test suite on every push and PR to `main`; the
live tls.peet.ws fingerprint matrix runs only on `workflow_dispatch` / the
weekly `fingerprint-cron.yml` (flaky network must not block PRs). The
`release-{node,python,crates}.yml` workflows publish on `v*` tags. `scripts/verify.sh`
is the local pre-PR/pre-tag gate that mirrors these checks — run it before
opening a PR and before tagging.

The pre-commit hook runs the offline Leyline workspace suite automatically. If
it fails, fix the cause - do not bypass the hook.

## The claim guard

`crates/leyline/tests/claim_guard.rs` scans the README and every public
`lib.rs` for marketing superlatives (`best`, `undetectable`,
`indistinguishable`, `perfect`, etc.) and fails the build if it finds them.
If a claim cannot be proved by a test in `TESTING.md`, it does not belong in
this repo. When in doubt, phrase things as *what Leyline does* rather than
*how it compares*.

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

## Releasing

A release ships from one `v*` tag: the Rust crate (crates.io), the Node addon
(npm/GitHub Packages), and the Python wheels. **[docs/RELEASING.md](docs/RELEASING.md)
is the source of truth** for the procedure and registry details — follow it, not
a hand-run `cargo publish`.

In brief:

1. Bump `workspace.package.version` in the root `Cargo.toml` and mirror it into
   `package.json` (`version` + the three `optionalDependencies`),
   `wrappers/python/pyproject.toml`, and the READMEs — CI's `version-parity` job
   enforces this. Add a `CHANGELOG.md` entry.
2. Run `scripts/verify.sh` (the local preflight) and merge to `main`.
3. `git tag -s vX.Y.Z -m "leyline vX.Y.Z" && git push origin vX.Y.Z`. The tag
   fires `release-node.yml` and `release-python.yml`; `release-crates.yml` is
   `workflow_dispatch`-only (public crates.io is a deliberate, manual go-live).

## Commit shape

Targeted commits. A refactor, a bug fix, a new
profile, and a docs pass are four commits, not one. The pre-commit hook
catches compile failures — a hook that fails means the commit did not happen
and `--amend` is the wrong tool. Fix the cause, re-stage, create a new commit.
