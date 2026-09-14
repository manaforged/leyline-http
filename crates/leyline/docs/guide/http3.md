# HTTP/3

The `http3` feature is on by default. It pulls in `leyline-quiche` and adds
`H3Config`, the `Http3` and `Race` protocol policies, and the `http3()` and
`race()` builder methods. Build with `default-features = false` to drop the
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

Set it with `protocol_policy(...)`, or with the shorthand `http1()`,
`http2()`, `http3()`, and `race()` builder methods.
`Session::protocol_policy()` reads it back.

```rust,no_run
use leyline::{ProtocolPolicy, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .chrome()
    .protocol_policy(ProtocolPolicy::Http3)
    .build()?;
assert_eq!(session.protocol_policy(), ProtocolPolicy::Http3);
# Ok(())
# }
```

Select a browser before forcing HTTP/3: the bare profile has no HTTP/3
configuration, so a forced `Http3` request from a bare session fails.
`Session::chrome()` and the other Chromium constructors select `Race`.

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
- No proxy applies.
- The scheme is `https`.

Otherwise the request falls back to `Auto`. When the race runs, the first
transport to hand back a connection carries the request, and the request is
sent once. If both fail, the `Auto` path runs.

## Transport configuration

`H3Config` holds the QUIC transport parameters and the HTTP/3 settings a
profile presents: the flow-control limits, `max_idle_timeout`,
`max_udp_payload_size`, `active_connection_id_limit`, the initial destination
connection ID length, the QPACK settings, `max_field_section_size`, and a cap
on the response body the HTTP/3 client accepts, streaming included.

`H3Config::for_family` selects the set for a profile family: `chromium`,
`gecko`, or `webkit`. Any other family is an `Kind::Config`.

```rust
use leyline::H3Config;

let chrome = H3Config::for_family("chromium").expect("chromium config");
assert_eq!(chrome.dcid_length, 8);
assert!(H3Config::for_family("nonesuch").is_err());
```

## QPACK

Every bundled profile advertises `qpack_max_table_capacity: 0` and
`qpack_blocked_streams: 0`. The dynamic table is not used in either
direction, so header fields are encoded against the static table and
literals only. This is a deliberate implementation limit: it keeps the
encoder deterministic, and it is a fingerprint difference from stacks whose
QPACK encoder uses a dynamic table. Version-specific captures would be
needed to establish full QPACK equivalence.

## No proxy support

HTTP/3 does not go through a proxy. A forced `Http3` request with a session
proxy or a per-request proxy fails with `Kind::Config`, telling you to use
`Auto` or `Http2`. Under `Race`, a proxied request is not raced and takes the
`Auto` path.

## Next

Read [Fingerprints](fingerprints.md).
