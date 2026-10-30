# TLS trust

Leyline verifies every server certificate. All trust settings live in one
`Tls` that you pass to `SessionBuilder::tls`: the roots, the
pins, the client certificate, and the TLS version floor. The browser profile
decides what the ClientHello looks like; trust settings decide which
certificates the session accepts.

## Choose the roots

By default a session trusts the system trust store and the files named by
`SSL_CERT_FILE` and `SSL_CERT_DIR` when they are set. `ca_file` adds a
PEM file and `ca_der` adds the DER bytes of one certificate; both add to
the default roots. Turn a default source off with `system_roots(false)` or
`env_roots(false)`. With both off, the session trusts only the roots you add.

```rust,no_run
use leyline::{Browser, Session, Tls};

# fn run() -> leyline::Result<()> {
let trust = Tls::new()
    .env_roots(false)
    .system_roots(false)
    .ca_file("/etc/myorg/ca.pem");

let chrome = Session::builder()
    .browser(Browser::default())
    .tls(trust.clone())
    .build()?;
let firefox = Session::builder()
    .browser(Browser::firefox())
    .tls(trust)
    .build()?;
# let _ = (chrome, firefox);
# Ok(())
# }
```

`Tls` is a value: build it once and pass a clone to each builder.
`tls` replaces the builder's trust settings, so put every setting in
one config. A file or certificate that cannot be parsed fails with
`Kind::Config`.

## Pin a certificate

`pin_sha256` takes the SHA-256 digest of the server's leaf
certificate in DER form. The handshake succeeds only when the chain verifies
and the leaf matches a pin. A pin adds no root.

```rust,no_run
# fn run() -> leyline::Result<()> {
let pin: [u8; 32] = [0; 32];
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .tls(leyline::Tls::new().pin_sha256(pin))
    .build()?;
# let _ = session;
# Ok(())
# }
```

Pins apply to every host the session contacts, so give a pinned host its own
session. Add a pin for each certificate you expect, including the next one
before a rotation. To read the digest of a live certificate, hash the
`peer_cert_der` field of `Response::tls()`.

A saved TLS session resumes only in a session that loaded the same pins,
roots, and client certificate, compared by content; any other session makes a
full handshake.

A pin failure is a `Kind::Tls` error.
With `Protocol::Http3` it is a `Kind::Http3` error with a message and
no source.

## Present a client certificate

For mutual TLS, give `client_cert` a PEM certificate chain and its
private key.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .tls(
        leyline::Tls::new()
            .client_cert("/etc/myorg/client.pem", "/etc/myorg/client.key"),
    )
    .build()?;
# let _ = session;
# Ok(())
# }
```

A session with pins or a client certificate cannot use an `https://` proxy.
A request through one fails with `Kind::Proxy`.

## Set a TLS version floor

`tls12_min()` refuses TLS 1.0 and 1.1, and `tls13_only()` refuses everything
below TLS 1.3. With neither, the floor is TLS 1.0. The handshake minimum is
the higher of the profile's minimum and the floor. A profile that declares no minimum starts at TLS 1.2.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .browser(leyline::Browser::Safari18).platform(leyline::Platform::MacOS)
    .tls(
        leyline::Tls::new().tls12_min(),
    )
    .build()?;
# let _ = session;
# Ok(())
# }
```

A floor above the profile's minimum changes the ClientHello, so the session
no longer matches its browser profile:

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
