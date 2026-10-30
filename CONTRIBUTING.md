# Contributing

Leyline does not accept pull requests before 1.0. To report a bug or request a feature, open an [issue](https://github.com/manaforged/leyline-http/issues). To report a vulnerability, follow [SECURITY.md](SECURITY.md).

## Build from source

You need rustup, Git, and a C and C++ compiler; on Windows, the MSVC build tools. `scripts/dev-setup.sh` (or `scripts/dev-setup.ps1` on Windows) checks the tools, builds the crate, and runs a short test pass.

```sh
git clone --recurse-submodules https://github.com/manaforged/leyline-http
cd leyline-http
cargo test -p leyline-http --features full
```

`scripts/verify.sh` runs the same gates as CI.
