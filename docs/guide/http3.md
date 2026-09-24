# HTTP/3

The `http3` feature is on by default. It pulls in `leyline-quiche` and adds
the `Http3` and `Race` protocol policies. Build with `default-features = false` to drop the
QUIC stack entirely.

## Protocol policy

`ProtocolPolicy` decides which transport a request uses.

| Variant | Behavior |
| --- | --- |
| `Auto` | HTTP/1.1 for `http://`, HTTP/2 for `https://`, falling back to HTTP/1.1 when ALPN does not negotiate `h2`. The default. |
| `Http1` | Force HTTP/1.1. |
| `Http2` | Force HTTP/2 over TLS. |
| `Http3` | Force HTTP/3 over QUIC. Needs the `http3` feature. |
| `Race` | Race QUIC against TCP and TLS for origins already known to speak HTTP/3. Needs the `http3` feature. |

Set it with `SessionBuilder::protocol`.

```rust,no_run
use leyline::{Browser, ProtocolPolicy, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::default())
    .protocol(ProtocolPolicy::Http3)
    .build()?;
# let _ = session;
# Ok(())
# }
```

Select a browser before forcing HTTP/3: the bare profile has no HTTP/3
configuration, so a forced `Http3` request from a bare session fails.
`Session::new()` selects `Race`. `Session::builder()` keeps `Auto`, so a
session built that way does not try HTTP/3. Add
`.protocol(ProtocolPolicy::Race)` to get the same policy.

A profile without an HTTP/3 fingerprint fails a forced `Http3` request with
`Kind::Config`.

## The Alt-Svc gate

Chrome does not open a speculative QUIC connection to an origin it has never
seen speak HTTP/3, and neither does Leyline. `Race` only races an origin the
pool already knows: one that advertised `h3` in an `Alt-Svc` response header,
or that already completed a QUIC handshake. Every other origin takes the
`Auto` path and pays one TCP handshake.

A request is raced only when all of these hold:

- The origin is known to speak HTTP/3.
- The request body is not a stream.
- The response is not streamed.
- No proxy applies, or the proxy is `socks5://` or `socks5h://`.
- The scheme is `https`.

Otherwise the request falls back to `Auto`. When the race runs, the first
transport to hand back a connection carries the request, and the request is
sent once. If both fail, the `Auto` path runs.

## Transport configuration

The `[h3]` table of a profile sets the QUIC transport parameters and the
HTTP/3 settings the profile presents: the flow-control limits,
`max_idle_timeout`, `max_udp_payload_size`, `active_connection_id_limit`, the
initial destination connection ID length, the QPACK settings,
`max_field_section_size`, and a cap on the response body the HTTP/3 client
accepts, streaming included. A profile with no `[h3]` table has no HTTP/3
transport.

`Session::new()` races HTTP/3 against HTTP/2 when the bundled profile's `[h3]`
table sets `race = true`. The bundled Chrome profiles do.

## QPACK

Every bundled profile advertises `qpack_max_table_capacity: 0` and
`qpack_blocked_streams: 0`. The dynamic table is not used in either
direction, so header fields are encoded against the static table and
literals only. Browsers use the QPACK dynamic table. Leyline does not, so an
observer can tell the two apart.

## Proxies

HTTP/3 goes through a `socks5://` or `socks5h://` proxy with the `socks`
feature. Leyline uses SOCKS5 `UDP ASSOCIATE` to relay the QUIC datagrams. See
[Proxies](proxies.md).

An `http://` or `https://` proxy cannot carry QUIC. A forced `Http3` request
through such a proxy fails with `Kind::Config`, telling you to use `Auto` or
`Http2`. Under `Race`, such a request is not raced and takes the `Auto` path.

## Next

Read [TLS trust](tls-trust.md).
