# TLS trust

By default, Leyline verifies every server certificate. This page covers which
roots it trusts, how to add your own, how to pin a certificate, how to present
a client certificate, and how to set a minimum TLS version.

The browser profile decides what the ClientHello looks like. Trust settings
decide which certificates the session accepts. Roots, pins, and client
certificates leave the ClientHello unchanged. These settings affect more than
certificate checks:

- `min_tls_version` can remove lower TLS versions from the ClientHello. See
  [Set a TLS version floor](#set-a-tls-version-floor).
- `danger_accept_invalid_certs` turns off TLS session resumption on TCP
  connections.
- A session with pins or a client certificate cannot use an `https://` proxy.
  A request through one fails with a `Kind::Proxy` error.

## Default roots

A session trusts two sources:

- The system trust store.
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

A file or certificate that cannot be parsed returns a `Kind::Config` error.

## Pin a certificate

`TlsTrustConfig::add_pinned_leaf_sha256` takes the SHA-256 digest of the server's leaf
certificate in DER form. The handshake succeeds only if the chain verifies and
the leaf matches one of the pins. A pin adds no root, so a leaf that matches a
pin but does not chain to a trusted root fails the handshake.

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

A pin failure is a `Kind::Tls` error, and `err.tls()` returns the `TlsError`
with the detail. With `ProtocolPolicy::Http3`, a pin failure is a `Kind::Http3`
error instead. It carries a message and no source, so `err.tls()` returns
`None`.

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

## Set a TLS version floor

`TlsTrustConfig::min_tls_version` sets a floor for the TLS version. It takes a
`TlsMinVersion`: `Tls10`, `Tls12`, or `Tls13`. The handshake minimum is the
higher of the profile's minimum and the floor. A profile that declares no
minimum starts at TLS 1.2. The default floor is `Tls10`, so a session with no
floor set uses the profile's minimum.

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

A floor at or below the profile's minimum changes nothing. A floor above it
changes the ClientHello, so the session no longer matches the browser that the
profile impersonates:

- Some Safari and CFNetwork profiles declare a TLS 1.0 minimum in the
  `min_tls_version` key of `[tls]`. For them, a `Tls12` floor removes TLS 1.0
  and 1.1 from the `supported_versions` extension.
- A `Tls13` floor removes TLS 1.2 from `supported_versions` in every profile.
  It also removes the TLS 1.2 cipher suites and the extensions that only TLS
  1.2 and earlier use.

Set a floor when a security policy forbids TLS 1.0 and 1.1. A server that
cannot negotiate the floor fails the handshake with a `Kind::Tls` error.
`Response::tls()` reports the negotiated version in its `version` field.

The floor applies to every TLS handshake over TCP, including the handshake with
an `https://` proxy. A proxy that cannot meet the floor fails the request with
a `Kind::Proxy` error. HTTP/3 always uses TLS 1.3, so the floor does not change
a QUIC handshake.

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

`tls_trust` replaces the builder's trust settings. Put every root, pin, client
certificate, and TLS version floor in one `TlsTrustConfig`.

## Turn verification off

`TlsTrustConfig::danger_accept_invalid_certs(true)` accepts any certificate on
TCP connections, which carry HTTP/1.1 and HTTP/2. Use it against a local test
server only.

On those connections it also skips pin checks and turns off TLS session
resumption. It has no effect on HTTP/3. QUIC always verifies the certificate
and applies pins, so `ProtocolPolicy::Http3` fails against a server with an
invalid certificate.

## Next

Read [Network](network.md).
