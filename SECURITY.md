# Security

## Reporting a vulnerability

Report vulnerabilities privately through
[GitHub security advisories](https://github.com/manaforged/leyline-http/security/advisories/new).
Do not open a public issue for a vulnerability.

You will get an acknowledgement within seven days. Fixes ship as a patch
release with a changelog entry that credits the reporter unless you ask
otherwise.

## Scope

- The `leyline-http` crate and the crates it bundles: `leyline-bssl`,
  `leyline-bssl-sys`, `leyline-bssl-tokio`, and `leyline-quiche`.
- The Node and Python wrappers under `wrappers/`. They are developed
  in-tree but are not part of the 0.1.0 release line and are not published
  to public registries.

## Out of scope

- Detection of the client by a remote site. Leyline mimics browsers on the
  wire, but a site that can tell the difference is not a security defect.
- Vulnerabilities in upstream BoringSSL or quiche that the bundled revision
  has not yet picked up. Open an issue so the revision can be bumped.

## Bundled BoringSSL and quiche

`leyline-bssl-sys` ships a prebuilt BoringSSL pinned to commit
`3a9254f16eda7a4c5d2260039ff23456a0a34de4`, the revision Chromium's DEPS
pinned at tag `150.0.7871.26`. See
[`crates/leyline-bssl-sys/PROVENANCE.md`](crates/leyline-bssl-sys/PROVENANCE.md)
for the carried patches and the rebuild steps.

A security fix released upstream in BoringSSL or quiche is picked up and
released in Leyline as soon as practicable. Each pickup gets a
`### Security` line in [CHANGELOG.md](CHANGELOG.md) that names the upstream
advisory and the new pinned revision, and the pinned revision is recorded in
`PROVENANCE.md`.
