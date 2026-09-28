# Contributing

## Before you open a pull request

1. Run the release gate:

   ```sh
   ./scripts/verify.sh --full
   ```

   It runs formatting, workspace Clippy with warnings denied, rustdoc with
   warnings denied, the workspace test suite, `cargo-deny`, package and
   consumer checks, and with `--full` the live fingerprint and smoke
   suites. The vendored `leyline-quiche` crate is excluded from the
   clippy, rustdoc, and test gates. The `leyline-bssl*` crates sit outside
   the workspace: the package gate packages them, and their own tests run
   with `cargo test --manifest-path crates/<crate>/Cargo.toml`. The gate needs Python 3, Node, rustup (the toolchain comes from
   `rust-toolchain.toml`), `cargo-deny`, and `cargo-truesight`. Every build compiles BoringSSL
   from source, so it also needs CMake 3.22 or later, a C and C++ compiler,
   and libclang; on Windows, the MSVC build tools and NASM. The live suites
   need network access.

   The BoringSSL patches live in `crates/leyline-bssl-sys/patches/` as a
   numbered series. The build applies them in order. List a new patch in
   `crates/leyline-bssl-sys/PROVENANCE.md`.

   Tests that need the network are marked `#[ignore]`. Run them with
   `cargo test -p leyline-http --features full -- --include-ignored`
   when your change touches a browser profile or the TLS, HTTP/2, or
   HTTP/3 wire path.

2. Keep the change to one topic. A bug fix, its regression test, and its
   changelog line are one pull request.

3. If the change adds, removes, or changes a public item, run
   `cargo truesight sync` and commit `api/` and `docs/`. The release gate
   runs `cargo truesight check`.

4. Add a line under `Unreleased` in `CHANGELOG.md` when the change is
   visible to a user of the crate.

## Pull request description

Pull requests are squash-merged. Fill in the template: the problem, the fix,
the user-visible impact, and the command you ran to test it. Commit messages
inside the branch can use any clear style.

## Browser profiles

A profile under `crates/leyline/profiles/` describes a real capture. Name
the browser build it was captured from in `captured_against`, set `capture`
to `browser`, `native`, `headless-shell`, `webview`, `emulator`, `inferred`, or
`self-referential`
(the build fails without it, and only `browser` profiles become
`Browser::latest`), and keep the
`ja4` and `akamai` recorded reference values next to the fields that produce
them. The offline conformance test gates every profile that fixes its
extension order. It puts each profile and dimension into one of five states:

- Gated: a fixed-order profile's reconstruction matches its reference value.
- Gated fail: that reconstruction differs from the reference value, so the
  test fails.
- Recon accurate: a reconstruction matches the reference value without a
  fixed order.
- Recon diverges: reconstructed and not matching, so `audit()` is not
  wire-exact there.
- Unanchored: no reference value, so nothing is claimed.

`api/leyline-http.txt`, `docs/reference/leyline-http/`, and `docs/llms.txt`
are generated. Regenerate them with `cargo truesight sync`. The release
check runs `cargo truesight check` and fails when a file is stale.
`scripts/profile-oneshot.sh` captures new browser builds and lands their
profiles; run it with `status` to see which versions are missing. It captures
from these sources only:

- Chrome: the Chrome binary named by `LEYLINE_CHROME`, the installed Google
  Chrome on macOS, or the stable package from Google's apt repository on
  Linux, checked against the SHA-256 in Google's signed index. The script
  refuses Chrome for Testing and `chrome-headless-shell`.
- Firefox: the official release build.
- Safari: Safari.app, driven by `safaridriver`. Mobile Safari has no automated
  capture.

The script checks the binary and the user agent it captured. It records the
exact build in `captured_against` and writes `capture = "browser"`. It exits
with an error instead of landing a profile from another source.

Before the first Safari capture, do these steps once:

1. In Safari, turn on Settings > Advanced > Show features for web developers.
2. In the Develop menu, select Allow Remote Automation.
3. Run `safaridriver --enable` and enter your password.

### Keeping profiles current

The `release-watch` workflow runs once a day. It compares each browser's
stable release (Chrome, Firefox, Brave, Edge, Opera, and Safari and iOS)
with the newest bundled profile, and checks the forked crates against upstream
security advisories:

```sh
python3 scripts/release-check.py
python3 scripts/upstream-check.py
```

Either command exits with status 1 when a new major release has no profile or
an advisory applies to a fork's base version. The workflow then fails, and
GitHub notifies the maintainers. A BoringSSL revision behind Chrome's, a newer
upstream release, or a vendor API error is only reported in the run log. A new release is captured on request with
the capture scripts above; nothing is captured on a schedule.

## Releasing

crates.io rejects a crate whose dependencies it does not have yet, so the
five crates publish in dependency order:

1. `leyline-bssl-sys`
2. `leyline-bssl`
3. `leyline-bssl-tokio` and `leyline-quiche`
4. `leyline-http`

`./scripts/verify.sh --full` passes on the release commit first. Its package
gate packages all five crates together.

## Style

Rust code is formatted by `rustfmt` with the repository defaults. Tests live
in `*_tests.rs` files or under `tests/`, not inside production modules.
