# Releasing

Leyline releases are deliberate direct operations. A Git tag does not build or
publish anything.

## Preflight

1. Update the workspace version in `Cargo.toml`, `package.json`, its optional
   dependency pins, and `wrappers/python/pyproject.toml`.
2. Update `CHANGELOG.md`.
3. Run `./scripts/verify.sh` from a clean exact-SHA checkout.
4. Tag only after the direct platform builds below pass.

## BoringSSL

Run `scripts/package-bssl.sh` on every supported target and review the generated
libraries and bindings. Do not copy an artifact between operating systems.

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

Run `cargo publish --dry-run` first, then publish dependencies before consumers.
The binding crates are `publish = false`. Public `cargo publish` requires the
authorized crates.io account and explicit operator intent.
