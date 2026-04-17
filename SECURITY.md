# Security policy

## Reporting a vulnerability

Report security issues through **GitHub private security advisories**:
<https://github.com/manaforged/leyline-http/security/advisories/new>.

Do not open a public issue for security reports. We will acknowledge receipt
within 72 hours and coordinate a fix and disclosure timeline with you.

## In scope

- Memory safety bugs in `leyline-*` crates or the vendored BoringSSL fork
- Incorrect TLS verification, certificate-chain handling, or hostname matching
- Request smuggling or header injection in the `leyline-h2` / HTTP/1.1 path
- Supply-chain compromise (unexpected crate version, tampered vendored source)
- CLI flags or library methods that escalate privilege or disclose secrets
  unexpectedly — in particular anything `danger_`-prefixed behaving outside its
  documented scope

## Not in scope

- **Fingerprint staleness.** A browser ships a new Chrome version and our
  profile drifts — that is a normal update, not a vulnerability. Open a
  regular issue.
- **Detection of Leyline by fingerprinting systems.** Leyline is a fingerprint
  control library, not a detection-evasion guarantee. Staying ahead of
  classifiers is an ongoing engineering concern, not a security contract.
- Vulnerabilities in third-party services (`tls.peet.ws`, Cloudflare QUIC
  endpoints) that Leyline uses only for its live test suite.

## Supported versions

Leyline is pre-1.0. Security fixes are only backported to the latest
`2.0.0-alpha.*` release. Pin a version and watch
[releases](https://github.com/manaforged/leyline-http/releases).

## Upstream coordination

The vendored TLS stack is derived from
[`0x676e67/boring2`](https://github.com/0x676e67/boring2), which in turn
tracks Google BoringSSL. When BoringSSL publishes a CVE, the pinned
commit in `vendor/leyline-ssl-sys/REVISION` is the source of truth for
whether Leyline is affected. See `CONTRIBUTING.md → "Syncing the
vendored TLS stack"` for the resync procedure. Full attribution is in
[`NOTICE`](NOTICE).
