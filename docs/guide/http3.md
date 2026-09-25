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
`max_idle_timeout`, `max_udp_payload_size`, `active_connection_id_limit`, and
a cap on the response body the HTTP/3 client accepts, streaming included. A
profile with no `[h3]` table has no HTTP/3 transport.

`Session::new()` races HTTP/3 against HTTP/2 when the bundled profile's `[h3]`
table sets `race = true`. The bundled Chrome profiles do.

## QUIC fingerprint

These `[h3]` keys shape the QUIC Initial and the HTTP/3 control stream. All
of them are optional; a profile without them keeps quiche's defaults.

| Key | Effect |
| --- | --- |
| `[h3.tls]` | A full `[tls]` table for the QUIC ClientHello. Without it, QUIC uses `[tls]`. Its `extension_permutation` may list `quic_transport_parameters` (57). `tls12_extensions = true` sends `extended_master_secret` and `renegotiation_info` although QUIC is TLS 1.3 only. |
| `dcid_length` | The initial destination connection ID length: a number, or `{ weights = [[length, weight], ...] }` for a weighted random length. |
| `scid_length` | The source connection ID length, 0 to 20. The default is `dcid_length`. |
| `transport_parameters` | The exact transport parameter list. `{ id = N }` sends the value from the `[h3]` fields. `{ id = N, varint = V }` and `{ id = N, hex = "..." }` send a fixed value. `{ id = 17, versions = { chosen, available, grease } }` sends `version_information`, with a GREASE version `first` or at a `random` position. `{ grease = { id_bits, max_len } }` sends a reserved parameter with a random ID `31 * N + 27` (N below `2^id_bits`) and 0 to `max_len` random bytes. |
| `transport_order` | `fixed`, `shuffle` (a new random order per connection), or `rotate` (the list rotated by a random offset). Entries with `pinned = true` keep their place. |
| `max_ack_delay_ms` | The `max_ack_delay` value, when the list sends parameter 11. |
| `settings` | The exact SETTINGS list, in order. `{ id, value }` sends a fixed setting. `{ grease = { id_bits, value_bits } }` sends a reserved setting `31 * N + 33` with a random value. The known settings in the list also configure the connection. |
| `control_grease_frame` | A reserved frame `{ id_bits, max_len }` sent on the control stream after SETTINGS. |
| `pseudo_order` | The pseudo-header order, as in `[h2]`. The default is `method`, `scheme`, `authority`, `path`. |
| `priority_update` | Sends a PRIORITY_UPDATE frame with the request's `priority` header value. |

A transport parameter list that sends `max_datagram_frame_size` (32) turns on
QUIC DATAGRAM receipt. Leyline reads RESET_STREAM_AT as RESET_STREAM and
ignores ACK_FREQUENCY and IMMEDIATE_ACK, so a profile can advertise
`reset_stream_at` and `min_ack_delay`.

## QPACK

The QPACK decoder keeps a dynamic table. A profile's `settings` list sets the
advertised `QPACK_MAX_TABLE_CAPACITY` and `QPACK_BLOCKED_STREAMS`. The decoder
applies the peer's encoder stream instructions, decodes field sections that
reference the table, holds a field section that needs inserts which have not
arrived yet (up to the advertised number of blocked streams), and sends
Section Acknowledgment, Insert Count Increment, and Stream Cancellation on the
decoder stream. The encoder does not use the dynamic table, as the peer
allows.

Profiles without a `settings` list use `qpack_max_table_capacity`,
`qpack_blocked_streams`, and `max_field_section_size` instead.

## Proxies

HTTP/3 goes through a `socks5://` or `socks5h://` proxy with the `socks`
feature. Leyline uses SOCKS5 `UDP ASSOCIATE` to relay the QUIC datagrams. See
[Proxies](proxies.md).

An `http://` or `https://` proxy cannot carry QUIC. A forced `Http3` request
through such a proxy fails with `Kind::Config`, telling you to use `Auto` or
`Http2`. Under `Race`, such a request is not raced and takes the `Auto` path.

## Next

Read [TLS trust](tls-trust.md).
