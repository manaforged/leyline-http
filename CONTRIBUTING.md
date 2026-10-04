# Contributing

Leyline does not accept external pull requests until its API is more stable.
Only accounts with write access can open a pull request. To report a bug or
request a feature, open an
[issue](https://github.com/manaforged/leyline-http/issues). To report a
vulnerability, follow [SECURITY.md](SECURITY.md).

The rest of this guide is the maintainers' workflow.

## Before you open a pull request

1. Run the gates. `./scripts/verify.sh` with no flag runs three of them:
   `comments` (the comment lint), `msrv` (`cargo check` for the workspace on
   the MSRV), and `package` (`cargo package` for the publishable crates, then
   a consumer check). The package gate builds from `HEAD`, so commit first.
   Before you open a pull request, run every gate:

   ```sh
   ./scripts/verify.sh --full
   ```

   `--full` adds formatting, workspace Clippy with warnings denied, the
   feature matrix, rustdoc with warnings denied, `cargo truesight check`, the
   mdbook build, the workspace test suite, the live fingerprint and smoke
   suites, `cargo-deny`, and the benchmark build. The `semver`,
   `external-types`, and `fuzz-replay` gates skip themselves when their tool
   is not installed. `--only` takes a comma-separated list of gates.

   The vendored `leyline-quiche` crate is excluded from the Clippy, rustdoc,
   and test gates. The `leyline-bssl*` crates sit outside the workspace: the
   package gate packages them, and the `subcrates` gate runs their own tests
   with `cargo test --manifest-path crates/<crate>/Cargo.toml`. The `release`
   gate checks a tag against the crate versions, the `=` pins between them,
   and a dated `CHANGELOG.md` heading.

   The gates need Python 3, Node, git, and rustup. The toolchain comes from
   `rust-toolchain.toml`, and the `msrv` gate also needs the MSRV toolchain.
   `--full` also needs `cargo-deny`, `cargo-truesight`, and `mdbook`. Install
   `cargo-truesight` with
   `cargo install --locked --git https://github.com/manaforged/truesight`.
   Every build compiles BoringSSL from source, so you also need CMake 3.22 or
   later and a C and C++ compiler; on Windows, the MSVC build tools and NASM.
   Regenerating the committed bindings with
   `scripts/regen-bssl-bindings.sh` also needs libclang. The live suites need
   network access.

   The BoringSSL patches live in `crates/leyline-bssl-sys/patches/` as a
   numbered series. The build applies them in order. List a new patch in
   `crates/leyline-bssl-sys/PROVENANCE.md`.

   Tests that need the network are marked `#[ignore]`. Run them with
   `cargo test -p leyline-http --features full,bench-internals -- --include-ignored`
   when your change touches a browser profile or the TLS, HTTP/2, or
   HTTP/3 wire path. Many test targets need the `bench-internals` feature,
   and `cargo test` skips them without it.

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
the browser build it was captured from in `captured_against`. Set `capture`
to `browser`, `native`, `headless-shell`, `webview`, `emulator`, `inferred`, or
`self-referential`; the build fails without it. `profiles/families.toml` lists
the capture kinds that `Browser::latest` accepts for each family: `browser` by
default, `browser` and `emulator` for Safari on iOS and OkHttp, and `native`
and `emulator` for CFNetwork. Keep the `ja4` and `akamai` recorded reference
values next to the fields that produce them.

The offline conformance test, `crates/leyline/tests/fingerprint_conformance.rs`,
compares each profile's HTTP/2 fingerprint with its `akamai` value and its JA4
with its `ja4` value. It puts each result into one of five states:

- Gated: the value matches its reference value. The test checks the HTTP/2
  fingerprint of every profile this way, and the JA4 of every profile that
  fixes its extension order.
- Gated fail: a value checked that way differs from its reference value, so
  the test fails.
- Recon accurate: the JA4 of a profile without a fixed extension order
  matches its reference value.
- Recon diverges: that JA4 differs from the reference value, so `audit()` is
  not identical to the bytes sent there.
- Unanchored: no reference value, so the test claims nothing.

`api/leyline-http.txt`, `docs/reference/leyline-http/`, and `docs/llms.txt`
are generated. Regenerate them with `cargo truesight sync`. The release
check runs `cargo truesight check` and fails when a file is stale.
`scripts/profile-oneshot.sh` captures new browser builds and lands their
profiles; run it with `status` to see which versions are missing. It captures
from these sources only:

- Chrome: the Chrome binary named by `LEYLINE_CHROME`, the installed Google
  Chrome on macOS, or the stable package from Google's apt repository on
  Linux. On Linux, the script compares the package with the SHA-256 in the
  repository's `Packages` index. It refuses Chrome for Testing and
  `chrome-headless-shell`.
- Firefox: the official release build from Mozilla's download server.
- Safari: Safari.app, driven by `safaridriver`. Mobile Safari has no automated
  capture.

The script runs Chrome with `--headless=new` and Firefox with `--headless`.
On Linux it also captures Firefox's HTTP/3 fingerprint headful under Xvfb
(`xvfb-run`), twice. It sets Chrome's user agent itself with `--user-agent`. Before it lands a
profile, it checks the Chrome binary's `--version` output or the Safari bundle
identifier. It also checks that the user agent the capture server saw names
the expected browser and contains no `Headless`; for Chrome, that check
confirms the script's own flag. The script records the exact build in
`captured_against` and writes `capture = "browser"`. It exits with an error
instead of landing a profile from another source.

Each capture is stored in `crates/leyline/profiles/captures/` with the client
address removed; the script refuses to store a capture that still holds a
public address outside `dst_ip`. To capture on several hosts, run
`scripts/profile-oneshot.sh firefox --capture-only` on each, copy the capture
files to one checkout, and land them with
`scripts/profile-oneshot.sh firefox --major 157 --land firefox-157.0`.

Landing starts from the previous profile of the family and changes only what
the captures show: the version, the user agents, the build in
`captured_against`, and any TLS list, JA4, or HTTP/2 value that differs. It
also adds the two rows in `docs/guide/profiles.md`, the README version range,
the changelog entry, and the row in `crates/leyline/tests/data/h3_qpack.toml`.
It does not change any other profile. It exits with status 2 and lists what a
person must review when the TLS extension order, JA4, HTTP/2, or HTTP/3
fingerprint changed in a way it cannot write, and always for the guide prose
and `cargo truesight sync` (or pass `--sync`).

Before the first Safari capture, do these steps once:

1. In Safari, turn on Settings > Advanced > Show features for web developers.
2. In the Develop menu, select Allow Remote Automation.
3. Run `safaridriver --enable` and enter your password.

### Keeping profiles current

The `release-watch` workflow runs daily. It compares each browser's stable
release (Chrome, Firefox, Brave, Edge, Opera, and Safari and iOS) with the
newest bundled profile, and checks the forked crates, the pinned BoringSSL
revision, and the OpenSSL advisories published since the `since` date in
`crates/leyline-bssl-sys/Cargo.toml`:

```sh
python3 scripts/release-check.py
python3 scripts/upstream-check.py
```

When a release has no profile, the BoringSSL revision differs from Chrome's,
or an advisory applies or is not yet reviewed, the scripts open an issue for
it, assigned to the maintainer, and exit with status 0. They exit with status
1 only when a check cannot run; the workflow then fails and opens a failure
issue. A new release is captured with the capture scripts above.

## Releasing

A release is a `v*` tag on a commit on `main`. Pushing the tag runs
`release.yml`:

1. Four gates run in parallel. `qualify` runs every gate in `ci.yml`.
   `hygiene` scans the tree and the commit messages since the previous `v*`
   tag for denied text. `matrix` runs the cross-platform matrix
   (`matrix.yml`) on Linux, macOS, and Windows. `release qualification`
   checks that the tag is on `main` and matches the crate versions, the `=`
   pins between the crates, and a dated `CHANGELOG.md` heading.
2. `package` starts only when all four gates pass. It packages the five
   crates and runs the consumer check in a job that holds no credentials.
3. `publish` waits for a maintainer to approve the `release` environment. It
   repackages the crates, checks that every `.crate` file matches the one
   `package` tested, publishes them in dependency order with a short-lived
   crates.io token from Trusted Publishing, and records build provenance for
   each file. No crates.io token is stored in the repository or on a
   maintainer's machine. It then creates the GitHub release from the
   changelog section, with the `.crate` files attached, and `book` deploys
   the guide.

crates.io publishes in dependency order: `leyline-bssl-sys`, `leyline-bssl`,
`leyline-bssl-tokio` and `leyline-quiche`, then `leyline-http`. `publish`
skips crates whose version is already on crates.io, so re-running a job that
stopped partway finishes the same release. A published version cannot be
replaced.

Hosted CI runs only for a release tag and once a night. Pull requests and
pushes to `main` start no workflow; the gates above run on the maintainer's
machines before a push. `nightly.yml` is the one scheduled run: the live
fingerprint and smoke suites against public servers, time-bounded fuzzing,
the cross-platform matrix (`matrix.yml`), and the release watch. It does not
block a release. `check.yml` and `scorecard.yml` run only when started by
hand.

## Style

Rust code is formatted by `rustfmt` with the repository defaults. Tests live
in the module's `tests.rs`, in a `*_tests.rs` file beside the module, or under
`tests/`, not inside production modules.
