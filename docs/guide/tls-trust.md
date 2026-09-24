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

All trust settings live in one `TlsTrustConfig`. Pass it to
`SessionBuilder::tls_trust`. Turn either source off on the config.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .tls_trust(
        leyline::TlsTrustConfig::new()
            .env_roots(false)
            .system_roots(false)
            .add_ca_file("/etc/myorg/ca.pem"),
    )
    .build()?;
# let _ = session;
# Ok(())
# }
```

With both sources off, the session trusts only the roots you add.

## Add a root

`TlsTrustConfig::add_ca_file` takes a PEM file. `add_ca_der` takes the DER
bytes of one certificate. Both add to the default roots; they do not
replace them.

A file or certificate that cannot be parsed returns a `Kind::Tls` error.

## Pin a certificate

`TlsTrustConfig::add_pinned_leaf_sha256` takes the SHA-256 digest of the server's leaf
certificate in DER form. The handshake succeeds only if the chain verifies and
the leaf matches one of the pins. A pin narrows trust; it never widens it.

```rust,no_run
# fn run() -> leyline::Result<()> {
let pin: [u8; 32] = [0; 32];
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .tls_trust(leyline::TlsTrustConfig::new().add_pinned_leaf_sha256(pin))
    .build()?;
# let _ = session;
# Ok(())
# }
```

Add one pin for each certificate you expect, including the next one before a
rotation. Pins apply to every host the session contacts, so give a pinned host
its own session.

To read the digest of a live certificate, hash
the `peer_cert_der` field of `Response::tls()`, which holds the same DER
bytes.

A pin failure is a `Kind::Tls` error. `err.tls()` returns the `TlsError` with
the detail.

## Present a client certificate

For mutual TLS, give `TlsTrustConfig::client_identity` a PEM certificate
chain and its private key.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .tls_trust(
        leyline::TlsTrustConfig::new()
            .client_identity("/etc/myorg/client.pem", "/etc/myorg/client.key"),
    )
    .build()?;
# let _ = session;
# Ok(())
# }
```

## Share one configuration

`TlsTrustConfig` is a value. Build it once and pass a clone to several
builders.

```rust,no_run
# fn run() -> leyline::Result<()> {
use leyline::TlsTrustConfig;

let trust = TlsTrustConfig::new()
    .env_roots(false)
    .add_ca_file("/etc/myorg/ca.pem");

let a = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .tls_trust(trust.clone())
    .build()?;
let b = leyline::Session::builder()
    .browser(leyline::Browser::latest(leyline::Family::Firefox))
    .tls_trust(trust)
    .build()?;
# let _ = (a, b);
# Ok(())
# }
```

`tls_trust` replaces the builder's trust settings. Put every root, pin, and
client certificate in one `TlsTrustConfig`.

## Turn verification off

`TlsTrustConfig::danger_accept_invalid_certs(true)` accepts any certificate. Use it against a
local test server only. It also makes pins meaningless.

## Next

Read [Network](network.md).
