# Releasing

Leyline releases are deliberate direct operations. A Git tag does not build or
publish anything.

## Preflight

1. Update the workspace version in `Cargo.toml`, `package.json`, its optional
   dependency pins, and `wrappers/python/pyproject.toml`.
2. Update `CHANGELOG.md`.
3. Run `./scripts/verify.sh` from a clean exact-SHA checkout. It checks the
   declared Rust 1.86 MSRV, packages every publishable Rust crate, and compiles
   a separate consumer against the extracted `leyline` package.
4. Tag only after the direct platform builds below pass.

## BoringSSL

Run `./scripts/verify.sh --bssl-source-build` on every supported target and
review the generated libraries and bindings. This command exercises the carried
BoringSSL source build before checking the resulting `leyline-bssl-sys` crate;
it intentionally updates the host bundle for review. Do not copy an artifact
between operating systems.

## Node

On Linux x64, Windows x64, and Mac arm64:

```bash
npm ci
npm run build
npm test
npm pack --dry-run
```

Collect the three reviewed `.node` artifacts, run `npm run artifacts`, inspect
the package contents, then publish explicitly with `npm publish` from the
authorized registry account.

## Python

On each supported platform:

```bash
python -m pip install 'maturin>=1.7,<2.0'
cd wrappers/python
maturin build --release --out dist
python -m pip install dist/leyline-*.whl
python -m pytest tests
```

Publish reviewed wheels explicitly with `maturin upload`. PyPI publication is a
public release; confirm the project license and ownership first.

## Rust crates

The binding crates are `publish = false`. The verification script stages the
committed tree as one temporary workspace and uses Cargo's multi-package
overlay, so unpublished internal dependencies can be normalized and checked
before the first crates.io release exists.

```bash
./scripts/verify.sh
```

Publish only with the authorized crates.io account and explicit operator intent,
in this dependency order: `leyline-bssl-sys`, `leyline-bssl`,
`leyline-bssl-tokio`, `leyline-quiche`, then `leyline`. Use the same manifest
boundaries for the excluded BoringSSL crates:

```bash
cargo publish --manifest-path crates/leyline-bssl-sys/Cargo.toml
cargo publish --manifest-path crates/leyline-bssl/Cargo.toml
cargo publish --manifest-path crates/leyline-bssl-tokio/Cargo.toml
cargo publish --manifest-path crates/leyline-quiche/Cargo.toml
cargo publish --manifest-path crates/leyline/Cargo.toml
```

Wait for each dependency version to appear in the crates.io index before moving
to its consumer. On a first release, the package-boundary smoke check above is
the rehearsal; a downstream `cargo publish --dry-run` cannot resolve internal
dependencies until they have been published.
