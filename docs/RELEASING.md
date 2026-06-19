# Releasing

Leyline ships four artifacts from one tag:

| Artifact | Registry (alpha) | Registry (public) | Workflow |
| -------- | ---------------- | ----------------- | -------- |
| Rust crate | — | crates.io | `release-crates.yml` |
| Node addon | GitHub Packages | npmjs.com | `release-node.yml` |
| Python wheels | artifacts only | PyPI | `release-python.yml` |

The package version is single-sourced from the workspace `version` in the root
`Cargo.toml`. `package.json`, `wrappers/python/pyproject.toml`, and the root
`optionalDependencies` are checked against it by the `version-parity` job in
`test.yml` on every PR — bump all of them together.

## Node.js prebuilt addon

`.github/workflows/release-node.yml` builds the N-API addon for the three
prebuilt-BoringSSL targets (`x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`,
`aarch64-apple-darwin`) and publishes:

- one **per-platform** binary package each (`@manaforged/leyline-linux-x64-gnu`,
  `-win32-x64-msvc`, `-darwin-arm64`) carrying just the `.node`, and
- the **main** `@manaforged/leyline` package (JS only), whose
  `optionalDependencies` pull the matching binary at install time.

No `BORING_BSSL_PATH` is needed: the static BoringSSL libs are vendored in-tree
under `crates/btls-sys/native/<target>/lib`, so each matrix leg links them
directly.

### Cutting a release

1. Bump the workspace version in `Cargo.toml`, and mirror it into
   `package.json` (`version` + the three `optionalDependencies`),
   `wrappers/python/pyproject.toml`, and both READMEs. CI's `version-parity`
   job fails the PR if any drift.
2. Merge to `main`.
3. Tag and push:
   ```bash
   git tag v1.0.0-alpha.2
   git push origin v1.0.0-alpha.2
   ```
   A `v*` tag fires `release-node.yml` and `release-python.yml` in
   **build-and-validate** mode — neither publishes. `release-node` builds all
   platforms and runs `npm publish --dry-run`; `release-python` builds and
   validates the wheels and sdist as artifacts. The tag does **not** fire
   `release-crates.yml` (manual-only). So cutting a tag publishes nothing,
   anywhere — it is a safe dress rehearsal. A real npm publish requires a manual
   dispatch with `dry_run` unchecked (see below).

### Publishing (and dry-running) the Node addon

Publishing is deliberate, never automatic. A `v*` tag — or a manual
`workflow_dispatch` left at the `dry_run: true` default — builds every platform
and runs `npm publish --dry-run` end to end, uploading nothing. To actually
publish to GitHub Packages, trigger `release-node.yml` via `workflow_dispatch`
with **`dry_run` unchecked**. (This mirrors `release-crates.yml` and
`release-python.yml`: no tag push can publish on its own.)

### Why GitHub Packages during the alpha

GitHub Packages (`npm.pkg.github.com`) serves **private** scoped packages with
no paid plan and authenticates via the built-in `GITHUB_TOKEN` — so the entire
pipeline runs while the repo and packages are private, with no external secret.
The `@manaforged` npm scope maps to the `manaforged` GitHub org automatically.

Consumers during the alpha need a read token (see the Node README's `.npmrc`
snippet). Publishing requires only `permissions: packages: write`, already set
in the workflow.

### Switching to the public npm registry

When going public on npmjs.com, the binary-distribution mechanism is unchanged;
only the publish target moves. In `release-node.yml`:

1. Point `actions/setup-node` at the public registry:
   ```yaml
   registry-url: "https://registry.npmjs.org"
   scope: "@manaforged"
   ```
2. Swap the publish token to an npm automation token:
   ```yaml
   NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
   ```
   (add `NPM_TOKEN` to repo secrets), and drop the `packages: write` permission.
3. Add `--access public` to each `npm publish` (scoped packages default to
   restricted).

Public consumers then `npm install @manaforged/leyline` with **no `.npmrc` and
no auth**. Reserve the `@manaforged` scope on npmjs.com before the first public
publish.

## Python wheels

`.github/workflows/release-python.yml` builds an **abi3** wheel (one wheel per
platform covers CPython ≥ 3.8 — pyo3's `abi3-py38` feature) for the three
prebuilt-BoringSSL targets, plus an sdist:

- `manylinux x86_64` (`x86_64-unknown-linux-gnu`)
- `windows x64` (`x86_64-pc-windows-msvc`)
- `macOS arm64` (`aarch64-apple-darwin`)

Each wheel is smoke-tested in CI (install + offline pytest, no Rust toolchain)
before upload.

### Private alpha: build + validate, no publish

PyPI has no private tier, so during the alpha a `v*` tag (or a manual run)
**builds and validates** every wheel and the sdist and uploads them as workflow
artifacts. Nothing is published. The team can pull the artifacts from the run.

### Public launch: PyPI via Trusted Publishing

The `pypi` job uploads to PyPI with **Trusted Publishing** (OIDC) — no API
token. Before the first publish:

1. Reserve the `leyline` name on PyPI.
2. Configure a [Trusted Publisher](https://docs.pypi.org/trusted-publishers/)
   for the project: owner `manaforged`, repo `leyline`, workflow
   `release-python.yml`, environment `pypi`.
3. Run the workflow via `workflow_dispatch` with `publish_pypi: true` (or change
   the `pypi` job's `if:` to trigger on tags once you're confident).

### Known limitation — glibc floor and sdist source builds

- Linux wheels are tagged at the CI runner's glibc (`manylinux_2_39` on the
  current `ubuntu-latest`/Ubuntu-24.04 runner — so the prebuilt linux wheel
  needs glibc ≥ 2.39; Debian 12 / Ubuntu 22.04 / RHEL 9 fall back to a source
  build). Lowering the floor (e.g. `manylinux_2_28`) requires building **both**
  the vendored BoringSSL static libs and the wheel inside an older manylinux
  container (the static libs, not maturin, carry the glibc symbols) — tracked
  follow-up.
- The sdist vendors the full Rust workspace but **not** the prebuilt BoringSSL
  libs, so a source install needs a Rust toolchain and `BORING_BSSL_PATH` (or
  the upstream CMake/Perl/Go build) — the same constraint as a crates.io source
  build. The wheels are the zero-compile path.

## Rust crates (crates.io)

crates.io is a **go-live** target, not part of the private alpha.
`.github/workflows/release-crates.yml` is **`workflow_dispatch`-only** — it is
never fired by a `v*` tag (unlike `release-node`/`release-python`), so the
alpha tag cut above cannot trigger a public crates.io publish by accident. It
publishes in dependency order:

```
leyline-quiche  →  leyline
```

The binding crates (`leyline-ffi`, `leyline-node`, `leyline-python`) are
`publish = false`. `btls-sys` is the workspace's local shim — excluded from the
workspace and patched in via `[patch.crates-io]`; a published `leyline` depends
on the real `btls`/`btls-sys 0.5.6` from crates.io.

### BoringSSL build on the verify path

crates.io verifies each crate by compiling the packaged tarball **in
isolation**, where the workspace `[patch.crates-io] btls-sys` does not apply. So
the verify build (and a consumer's `cargo add leyline`) source-builds BoringSSL
and needs `cmake`, `perl`, `go`, and `clang`/`libclang` — the workflow installs
them. The zero-compile-for-Rust path (a prebuilt-BoringSSL `btls` fork) is a
tracked follow-up.

### docs.rs

`crates/leyline/Cargo.toml` carries `[package.metadata.docs.rs]` pinning a
single target. docs.rs has no workspace patch either, so it also source-builds
BoringSSL; the build succeeds only if the docs.rs builder provides
cmake/perl/go. Until the prebuilt-BoringSSL `btls` fork lands, expect docs.rs
to need that toolchain (or host the rustdoc output ourselves).

### First release

Run `release-crates.yml` via `workflow_dispatch`. A `--dry-run` of `leyline`
fails before `leyline-quiche` is on crates.io (it can't resolve the
dependency). For the first cut, dispatch with `dry_run: false` (a real publish),
or publish `leyline-quiche` first and re-run. Subsequent dry-runs work normally.
Add `CRATES_IO_TOKEN` to repo secrets before a real publish.
