# Security

## Reporting a vulnerability

Report vulnerabilities privately through
[GitHub security advisories](https://github.com/manaforged/leyline-http/security/advisories/new).
Do not open a public issue for a vulnerability.

You will get an acknowledgement within 7 days of your report.

For a high or critical issue, the target is a fix or a mitigation within 30
days of the acknowledgement. The fix ships as a patch release. Its
[CHANGELOG.md](CHANGELOG.md) entry has a `### Security` line that describes
the issue and credits the reporter, unless you ask otherwise.

One maintainer reviews reports. If a fix will miss the 30-day target, the
maintainer tells you the new date in the advisory thread.

## Supported versions

Security fixes go to the latest 0.1.x release.

## Scope

- The `leyline-http` crate and the crates it bundles: `leyline-bssl`,
  `leyline-bssl-sys`, `leyline-bssl-tokio`, and `leyline-quiche`.

## Out of scope

- Detection of the client by a remote site. Leyline mimics browsers on the
  wire, but a site that can tell the difference is not a security defect.
- Vulnerabilities in upstream BoringSSL or quiche. Report them to the
  upstream project.

## Bundled BoringSSL and quiche

`leyline-bssl-sys` builds BoringSSL from source, pinned to commit
`ac39ea6853833c1f18fd23614091d11855e71752`, the `boringssl_revision` in
Chromium's DEPS at tag `154.0.8037.58`. See
[`crates/leyline-bssl-sys/PROVENANCE.md`](crates/leyline-bssl-sys/PROVENANCE.md)
for the carried patches and the build notes.

A daily workflow checks the forked crates and the pinned BoringSSL revision
against upstream security advisories. `PROVENANCE.md` records the pinned
revision.
