# TLS trust

Leyline verifies every server certificate. This page covers which roots it
trusts, how to add your own, how to pin a certificate, and how to present a
client certificate.

The browser profile decides what the ClientHello looks like. Trust settings
decide which certificates you accept. The two do not interact: changing trust
does not change your fingerprint.

## Default roots

A session trusts two sources:

- The system trust store (feature `system-trust`, on by default).
- The files named by the `SSL_CERT_FILE` and `SSL_CERT_DIR` environment
  variables, when they are set.

Turn either source off on the builder.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .chrome()
    .without_env_roots()
    .without_system_roots()
    .add_root_certificate_file("/etc/myorg/ca.pem")
    .build()?;
# let _ = session;
# Ok(())
# }
```

With both sources off, the session trusts only the roots you add.

## Add a root

`add_root_certificate_file` takes a PEM file. `add_root_certificate_der` takes
the DER bytes of one certificate. Both add to the default roots; they do not
replace them.

A file or certificate that cannot be parsed returns a `Kind::Tls` error.

## Pin a certificate

`add_pinned_leaf_sha256` takes the SHA-256 digest of the server's leaf
certificate in DER form. The handshake succeeds only if the chain verifies and
the leaf matches one of the pins. A pin narrows trust; it never widens it.

```rust,no_run
# fn run() -> leyline::Result<()> {
let pin: [u8; 32] = [0; 32];
let session = leyline::Session::builder()
    .chrome()
    .add_pinned_leaf_sha256(pin)
    .build()?;
# let _ = session;
# Ok(())
# }
```

Add one pin for each certificate you expect, including the next one before a
rotation. Pins apply to every host the session contacts, so give a pinned host
its own session.

To read the digest of a live certificate, hash
`Response::tls_peer_certificate()`, which returns the same DER bytes.

A pin failure is a `Kind::Tls` error. `err.tls()` returns the `TlsError` with
the detail.

## Present a client certificate

For mutual TLS, give the builder a PEM certificate chain and its private key.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .chrome()
    .client_identity_files("/etc/myorg/client.pem", "/etc/myorg/client.key")
    .build()?;
# let _ = session;
# Ok(())
# }
```

## Share one configuration

`TlsTrustConfig` carries the same settings as a value. Build it once and pass
it to several builders with `SessionBuilder::tls_trust`.

```rust,no_run
# fn run() -> leyline::Result<()> {
use leyline::TlsTrustConfig;

let trust = TlsTrustConfig::new()
    .without_env_roots()
    .add_ca_file("/etc/myorg/ca.pem");

let a = leyline::Session::builder().chrome().tls_trust(trust.clone()).build()?;
let b = leyline::Session::builder().firefox().tls_trust(trust).build()?;
# let _ = (a, b);
# Ok(())
# }
```

`tls_trust` replaces the builder's trust settings, so call it before the
`add_*` shortcuts, not after.

## Turn verification off

`danger_accept_invalid_certs(true)` accepts any certificate. Use it against a
local test server only. It also makes pins meaningless.

## Next

Read [Network](network.md).
