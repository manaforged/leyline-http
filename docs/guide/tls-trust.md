# TLS trust

Leyline verifies every server certificate by default. All trust settings live
in one `Tls` value passed to `SessionBuilder::tls`. The browser profile sets
what the ClientHello looks like; `Tls` decides which certificates, client
identities, ECH configs, and TLS versions the session accepts.

## `Tls` setters at a glance

| Setter | Default | Effect |
| --- | --- | --- |
| `system_roots(bool)` | `true` | Trust the OS certificate store |
| `env_roots(bool)` | `true` | Trust `SSL_CERT_FILE` and `SSL_CERT_DIR` when set |
| `ca_file(path)` | none | Add CA certificates from a PEM file |
| `ca_pem(impl Into<Vec<u8>>)` | none | Add CA certificates from in-memory PEM bytes (call `.to_vec()` on `Bytes`) |
| `ca_der(impl Into<Vec<u8>>)` | none | Add one CA certificate from DER bytes |
| `pin_cert_sha256([u8; 32])` | none | Require the verified leaf certificate's DER SHA-256 to match a pin |
| `client_cert(cert_path, key_path)` | none | Mutual TLS from PEM files (leaf-first chain + unencrypted key) |
| `client_cert_pem(chain, key)` | none | Mutual TLS from in-memory PEM bytes (`impl Into<Vec<u8>>`) |
| `client_cert_pkcs12(der, password)` | none | Mutual TLS from a DER PKCS#12 archive (`impl Into<Vec<u8>>`, `impl Into<String>`) |
| `ech(bool)` | `false` | Fetch RFC 9460 `HTTPS`/SVCB records via DoH or custom resolver for ECH |
| `ech_config_list(impl Into<Vec<u8>>)` | none | Use a raw RFC 9460 `ECHConfigList` (takes precedence over `ech(true)`) |
| `min_version(TlsVersion)` | `Tls10` floor (plain sessions enforce **TLS 1.2**) | Minimum TLS version floor (`Tls10`, `Tls12`, or `Tls13`) |
| `danger_accept_invalid_hostnames(bool)` | `false` | Skip hostname matching while keeping CA and pin checks |
| `danger_accept_invalid_certs(bool)` | `false` | Accept any server certificate (skips CA, pins, and resumption) |

`Tls` is a `Clone` value; calling `SessionBuilder::tls` replaces the previous
`Tls` config, so combine all trust settings in one value. Unreadable files or
invalid certificates fail `build()` with `Kind::Config`.

## Choose the roots

By default a session trusts the system store and `SSL_CERT_FILE` /
`SSL_CERT_DIR`. `ca_file`, `ca_pem`, and `ca_der` add roots on top of the
defaults; turn defaults off with `system_roots(false)` and `env_roots(false)`
to trust only the roots you supply.

```rust,no_run
use leyline::{Browser, Session, Tls};

# fn run() -> leyline::Result<()> {
let trust = Tls::new()
    .env_roots(false)
    .system_roots(false)
    .ca_file("/etc/myorg/ca.pem");

let api = Session::builder().tls(trust.clone()).build()?;
let chrome = Session::builder()
    .browser(Browser::Chrome158)
    .tls(trust)
    .build()?;
# let _ = (api, chrome);
# Ok(())
# }
```

## Pin a certificate

`pin_cert_sha256` takes the SHA-256 digest of the server's whole leaf
certificate in DER form (not a public-key pin). The handshake succeeds only
when the chain verifies **and** the leaf matches a pin; a pin adds no root.

```rust,no_run
# fn run() -> leyline::Result<()> {
let pin: [u8; 32] = [0; 32];
let session = leyline::Session::builder()
    .tls(leyline::Tls::new().pin_cert_sha256(pin))
    .build()?;
# let _ = session;
# Ok(())
# }
```

- Pins apply to every host the session contacts, so give a pinned host its own session and include the next rotation pin before cutover. Hash `Response::peer_certificate()` to inspect a live leaf's DER digest.
- A saved TLS session resumes only when loaded into a session with matching roots, pins, and client certificate.
- A pin failure is `Kind::Tls` across HTTP/1.1, HTTP/2, and HTTP/3.

## Present a client certificate

For mutual TLS, pass PEM files to `client_cert(chain_path, key_path)`,
in-memory PEM bytes to `client_cert_pem(chain_pem, key_pem)`, or a DER PKCS#12
archive and password to `client_cert_pkcs12(der, password)`. Each call replaces
any earlier client certificate, and `Debug` output never prints key bytes or
passwords.

```rust,no_run
# fn run(chain_pem: Vec<u8>, key_pem: Vec<u8>, p12_der: Vec<u8>) -> leyline::Result<()> {
let from_files = leyline::Session::builder()
    .tls(leyline::Tls::new().client_cert("/etc/myorg/client.pem", "/etc/myorg/client.key"))
    .build()?;
let from_pem = leyline::Session::builder()
    .tls(leyline::Tls::new().client_cert_pem(chain_pem, key_pem))
    .build()?;
let from_p12 = leyline::Session::builder()
    .tls(leyline::Tls::new().client_cert_pkcs12(p12_der, "archive-password"))
    .build()?;
# let _ = (from_files, from_pem, from_p12);
# Ok(())
# }
```

- `client_cert_pkcs12` sends the leaf and all extra intermediate chain certificates in the archive (`""` works for passwordless archives). It does **not** add bundled CA certs to the trust store — still pass `ca_file`/`ca_pem` for private CAs.
- Certificates and CA files are read once at `build()`; to rotate expiring certs in a long-lived service, build a fresh `Session` and swap it (for example via `ArcSwap`).
- Mutual TLS and pins work through `http://` `CONNECT` proxies, but **not** through `https://` proxies (requests fail with `Kind::Proxy`; see [Proxies](proxies.md#socks5-and-https-proxies)).

## Encrypt SNI with ECH

Pass a raw RFC 9460 `ECHConfigList` to `Tls::ech_config_list`, or pair
`SessionBuilder::doh` (or a custom `Resolver::resolve_ech`) with
`Tls::new().ech(true)` to encrypt the TLS 1.3 ClientHello SNI (hiding the
target domain from passive network observers). If a server rejects a stale
config and sends `retry_configs`, Leyline retries once automatically.
`Response::ech_accepted()` reports whether ECH succeeded.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .doh("https://cloudflare-dns.com/dns-query")
    .tls(leyline::Tls::new().ech(true))
    .build()?;
# let _ = session;
# Ok(())
# }
```

- `ech_config_list` takes precedence over `ech(true)` when both are set.
- `SessionBuilder::doh` inherits `Tls` CA roots (`system_roots`, `ca_pem`, `ca_der`), while stripping client certs and pins. If you set `system_roots(false)` with an internal CA, also include the DoH provider's CA or point `doh` at an internal resolver.
- ECH works over TCP (HTTP/1.1, HTTP/2), HTTP/3, and proxies; it is skipped on `unix_socket` connections.

## Set a TLS version floor

`min_version` accepts `TlsVersion::Tls10` (default), `TlsVersion::Tls12`, or
`TlsVersion::Tls13`. The handshake minimum is the higher of the profile's
minimum and the floor (a profile with no minimum starts at TLS 1.2).

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .browser(leyline::Browser::Safari18).platform(leyline::Platform::MacOS)
    .tls(leyline::Tls::new().min_version(leyline::TlsVersion::Tls12))
    .build()?;
# let _ = session;
# Ok(())
# }
```

Raising the floor above a profile's minimum alters its ClientHello (for example,
removing TLS 1.0/1.1 from Safari/CFNetwork profiles or stripping TLS 1.2 cipher
suites and extensions under `Tls13`). A server that cannot meet the floor fails
with `Kind::Tls` (`Kind::Proxy` for an `https://` proxy). HTTP/3 always uses
TLS 1.3.

## Turn verification off

- `danger_accept_invalid_hostnames(true)` skips hostname matching while still verifying CA roots and leaf pins.
- `danger_accept_invalid_certs(true)` accepts any server certificate across HTTP/1.1, HTTP/2, and HTTP/3, skipping pins and TLS session resumption.
- Both options disable HSTS recording (see [Network](network.md#hsts)). Use them against controlled or local servers only.
