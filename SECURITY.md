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

## Scope

- The `leyline-http` crate and the crates it bundles: `leyline-bssl`,
  `leyline-bssl-sys`, `leyline-bssl-tokio`, and `leyline-quiche`.

## Out of scope

- Detection of the client by a remote site. Leyline mimics browsers on the
  wire, but a site that can tell the difference is not a security defect.
- Vulnerabilities in upstream BoringSSL or quiche that the bundled revision
  has not yet picked up. Open an issue so the revision can be bumped.

## Bundled BoringSSL and quiche

`leyline-bssl-sys` builds BoringSSL from source, pinned to commit
`3a9254f16eda7a4c5d2260039ff23456a0a34de4`, the revision Chromium's DEPS
pinned at tag `150.0.7871.26`. See
[`crates/leyline-bssl-sys/PROVENANCE.md`](crates/leyline-bssl-sys/PROVENANCE.md)
for the carried patches and the rebuild steps.

A security fix released upstream in BoringSSL or quiche is picked up and
released in the next Leyline patch release. Each pickup gets a
`### Security` line in [CHANGELOG.md](CHANGELOG.md) that names the upstream
advisory and the new pinned revision, and the pinned revision is recorded in
`PROVENANCE.md`.
