# TLS trust

Leyline verifies every server certificate. All trust settings live in one
`TlsTrustConfig` that you pass to `SessionBuilder::tls_trust`: the roots, the
pins, the client certificate, and the TLS version floor. The browser profile
decides what the ClientHello looks like; trust settings decide which
certificates the session accepts.

## Choose the roots

By default a session trusts the system trust store and the files named by
`SSL_CERT_FILE` and `SSL_CERT_DIR` when they are set. `add_ca_file` adds a
PEM file and `add_ca_der` adds the DER bytes of one certificate; both add to
the default roots. Turn a default source off with `system_roots(false)` or
`env_roots(false)`. With both off, the session trusts only the roots you add.

```rust,no_run
use leyline::{Browser, Family, Session, TlsTrustConfig};

# fn run() -> leyline::Result<()> {
let trust = TlsTrustConfig::new()
    .env_roots(false)
    .system_roots(false)
    .add_ca_file("/etc/myorg/ca.pem");

let chrome = Session::builder()
    .browser(Browser::default())
    .tls_trust(trust.clone())
    .build()?;
let firefox = Session::builder()
    .browser(Browser::latest(Family::Firefox))
    .tls_trust(trust)
    .build()?;
# let _ = (chrome, firefox);
# Ok(())
# }
```

`TlsTrustConfig` is a value: build it once and pass a clone to each builder.
`tls_trust` replaces the builder's trust settings, so put every setting in
one config. A file or certificate that cannot be parsed fails with
`Kind::Config`.

## Pin a certificate

`add_pinned_leaf_sha256` takes the SHA-256 digest of the server's leaf
certificate in DER form. The handshake succeeds only when the chain verifies
and the leaf matches a pin. A pin adds no root.

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

Pins apply to every host the session contacts, so give a pinned host its own
session. Add a pin for each certificate you expect, including the next one
before a rotation. To read the digest of a live certificate, hash the
`peer_cert_der` field of `Response::tls()`.

A pin failure is a `Kind::Tls` error, and `err.tls()` returns the `TlsError`.
With `ProtocolPolicy::Http3` it is a `Kind::Http3` error with a message and
no source, so `err.tls()` returns `None`.

## Present a client certificate

For mutual TLS, give `client_identity` a PEM certificate chain and its
private key.

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

A session with pins or a client certificate cannot use an `https://` proxy.
A request through one fails with `Kind::Proxy`.

## Set a TLS version floor

`min_tls_version` takes a `TlsMinVersion`: `Tls10` (the default), `Tls12`,
or `Tls13`. The handshake minimum is the higher of the profile's minimum and
the floor. A profile that declares no minimum starts at TLS 1.2.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .browser(leyline::Browser::Safari18)
    .platform(leyline::Platform::MacOS)
    .tls_trust(
        leyline::TlsTrustConfig::new().min_tls_version(leyline::TlsMinVersion::Tls12),
    )
    .build()?;
# let _ = session;
# Ok(())
# }
```

A floor above the profile's minimum changes the ClientHello, so the session
no longer matches the browser it impersonates:

- Some Safari and CFNetwork profiles declare a TLS 1.0 minimum. A `Tls12`
  floor removes TLS 1.0 and 1.1 from `supported_versions`.
- A `Tls13` floor removes TLS 1.2 from `supported_versions` in every
  profile, with the TLS 1.2 cipher suites and the extensions only TLS 1.2
  and earlier use.

A server that cannot meet the floor fails the handshake with `Kind::Tls`;
an `https://` proxy that cannot meet it fails with `Kind::Proxy`.
`Response::tls()` reports the negotiated version in `version`. HTTP/3 always
uses TLS 1.3, so the floor does not change a QUIC handshake.

## Turn verification off

`danger_accept_invalid_certs(true)` accepts any certificate on TCP
connections (HTTP/1.1 and HTTP/2). Use it against a local test server only.
On those connections it also skips pin checks and turns off TLS session
resumption. It has no effect on HTTP/3: QUIC always verifies the certificate
and applies pins.

## Next

Read [Network](network.md).
