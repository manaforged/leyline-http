# HTTP/3

The `http3` feature is on by default. It adds the `Http3` and `Race`
protocol policies and the QUIC stack. Build with `default-features = false`
to drop the QUIC stack. This page covers when a request uses HTTP/3, the
QUIC fingerprint keys of a profile, and HTTP/3 through a proxy.

## Protocol policy

`SessionBuilder::protocol` sets the `ProtocolPolicy`.

| Variant | Behavior |
| --- | --- |
| `Auto` | HTTP/1.1 for `http://`. For `https://`, one TLS handshake that offers the profile's ALPN list; the server's choice decides HTTP/2 or HTTP/1.1 |
| `Http1` | Force HTTP/1.1 |
| `Http2` | Force HTTP/2 over TLS |
| `Http3` | Force HTTP/3 over QUIC. Needs the `http3` feature |
| `Race` | Race QUIC against TCP and TLS for origins known to speak HTTP/3. Needs the `http3` feature |

Without a `protocol` call, a session selects `Race` when its profile sets
`race = true` in `[h3]`, as the bundled Chrome profiles do, and `Auto`
otherwise. `Session::browser(Browser::default())` selects `Race`;
`Session::new()` and a build without the `http3` feature select `Auto`.

Keep that default for a browser session: `Race` uses HTTP/3 where a browser
would and falls back to TCP. `Http3` forces QUIC on every request, the first
one included, and fails where QUIC is blocked; use it only to test that an
origin speaks HTTP/3. `build()` fails with `Kind::Config` when `Http3` or
`Race` meets a profile with no `[h3]` table, such as the bare profile.

## The Alt-Svc gate

The first request to an origin never uses HTTP/3. As in Chrome, `Race` opens
no speculative QUIC connection: it races only an origin that advertised `h3`
in an `Alt-Svc` response header. The first request goes over HTTP/2 or
HTTP/1.1 and records the header; the next one can race.
`Response::version()` returns the `HttpVersion` that carried each response.

```rust,no_run
use leyline::{Browser, HttpVersion, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::browser(Browser::default());
let url = "https://www.cloudflare.com/";

let first = session.get(url).send().await?;
println!("first: {:?}", first.version());

let second = session.get(url).send().await?;
if second.version() == HttpVersion::Http3 {
    println!("second: HTTP/3");
}
# Ok(())
# }
```

The second request can still use HTTP/2: the first transport to hand back a
connection carries the request, and a pooled HTTP/2 connection is often
ready first. The request is sent once. If both transports fail, the `Auto`
path runs. [`examples/http3.rs`](../../crates/leyline/examples/http3.rs)
runs this sequence.

A request is raced only when all of these hold; otherwise it takes the
`Auto` path:

- The scheme is `https` and the origin is known to speak HTTP/3.
- Neither the request body nor the response is streamed.
- No proxy applies, or the proxy is `socks5://` or `socks5h://` and the
  `socks` feature (not a default feature) is on.

When an HTTP/3 connection to an origin fails, `Race` skips HTTP/3 for that
origin and proxy pair for 5 minutes. A successful HTTP/3 connection to the
pair ends the skip.

`Session::state()` saves the known origins, so a restored session can race
on its first request. See
[Accounts](accounts.md#keep-the-connection-state).

### How Alt-Svc is read

Only an `h3=` entry for the same port counts, and its host must be empty or
equal to the origin's host. The origin is kept until the longest `ma` of the
matching entries, less the response's `Age`, ends; without `ma`, 24 hours.
Each `Alt-Svc` value replaces the one before it: `ma=0` on every matching
entry, `clear`, or a value with no usable `h3=` entry removes the origin. An
entry whose `ma` is not a number is ignored. The pool keeps at most 1024
origins and drops the one that expires first to make room.

## Body limits and cancellation

HTTP/3 uses `CompressionConfig::max_body_size`, as HTTP/1.1 and HTTP/2 do.
It caps a buffered body and fails with `Kind::Body`; a `.stream()` response
has no cap. See [Responses](responses.md#bodies).

A request that ends early resets its stream with the code Chrome sends:

| Cause | Stream halves reset | Code |
| --- | --- | --- |
| The caller drops the response or its body | Both | `H3_REQUEST_CANCELLED` |
| A buffered body passes `max_body_size` | Both | `H3_REQUEST_CANCELLED` |
| The request body cannot be sent | Both | `H3_REQUEST_CANCELLED` |
| The response ends, or the peer resets it, before the request body is sent | The request half | `H3_REQUEST_CANCELLED` |
| The response is malformed | Both | `H3_GENERAL_PROTOCOL_ERROR` |
| A request body stream fails | Both | `H3_GENERAL_PROTOCOL_ERROR` |

When the pool drops an idle connection, Leyline sends no CONNECTION_CLOSE
frame, as Chrome does not.

## QUIC fingerprint

The `[h3]` table of a profile sets the QUIC transport parameters and the
HTTP/3 settings: the flow-control limits, `max_idle_timeout_secs`,
`max_udp_payload_size`, and `active_connection_id_limit`. These keys shape
the QUIC Initial and the HTTP/3 control stream. `dcid_length` is required;
without the other keys, a profile keeps quiche's defaults.

| Key | Effect |
| --- | --- |
| `[h3.tls]` | A full `[tls]` table for the QUIC ClientHello. Without it, QUIC uses `[tls]`. Its `extension_permutation` may list `quic_transport_parameters` (57). `permute_extensions = true` with `extension_tail = [57, 65037]` shuffles the extensions on each connection and keeps the listed IDs last, in that order. `tls12_extensions = true` sends `extended_master_secret` and `renegotiation_info` although QUIC is TLS 1.3 only |
| `dcid_length` | The initial destination connection ID length: a number, or `{ weights = [[length, weight], ...] }` for a weighted random length |
| `scid_length` | The source connection ID length, 0 to 20. The default is `dcid_length` |
| `transport_parameters` | The exact transport parameter list. `{ id = N }` sends the value from the `[h3]` fields. `{ id = N, varint = V }` and `{ id = N, hex = "..." }` send a fixed value. `{ id = 17, versions = { chosen, available, grease } }` sends `version_information`, with a GREASE version `first` or at a `random` position. `{ grease = { id_bits, max_len } }` sends a reserved parameter with a random ID `31 * N + 27` (N below `2^id_bits`) and 0 to `max_len` random bytes |
| `initial_datagram_size` | The size of each client datagram that carries an Initial packet during the handshake, 1200 to `max_udp_payload_size`, at most 1232 over IPv6. Each first-flight Initial is padded with PADDING frames to this size; after a loss timeout with no reply, the size falls back to 1200. Without it, Initials are not padded and the datagram is filled with zeros to 1200 bytes |
| `initial_crypto_split` | `fill` (default) or `even`. With `even`, the first Initial packet carries an even share of the ClientHello and no other packet shares its datagram; the next carries the rest |
| `initial_crypto_reorder` | `none` (default) or `sni_midpoint`, which needs `initial_crypto_split = "even"`. The ClientHello is cut at the midpoint of the server name. The first Initial carries the end of the ClientHello in one CRYPTO frame, then the start up to the cut in a second; the next Initial carries the middle |
| `transport_order` | `fixed`, `shuffle` (a new random order per connection), or `rotate` (the list rotated by a random offset). Entries with `pinned = true` keep their place |
| `max_ack_delay_ms` | The `max_ack_delay` value, when the list sends parameter 11 |
| `settings` | The exact SETTINGS list, in order. `{ id, value }` sends a fixed setting. `{ grease = { id_bits, value_bits } }` sends a reserved setting `31 * N + 33` with a random value. Known settings in the list also configure the connection |
| `control_grease_frame` | A reserved frame `{ id_bits, max_len }` sent on the control stream after SETTINGS |
| `pseudo_order` | The pseudo-header order, as in `[h2]`. The default is `method`, `scheme`, `authority`, `path` |
| `priority_update` | Sends a PRIORITY_UPDATE frame with the request's `priority` header value |

Leyline speaks QUICv1 and QUICv2 (RFC 9369). It sends its first Initial in
the `chosen` version of `version_information` and follows a server's
compatible version negotiation (RFC 9368) to another version in `available`.

A transport parameter list that sends `max_datagram_frame_size` (32) turns on
QUIC DATAGRAM receipt. Leyline reads RESET_STREAM_AT as RESET_STREAM and
ignores ACK_FREQUENCY and IMMEDIATE_ACK, so a profile can advertise
`reset_stream_at` and `min_ack_delay`.

### QPACK

The QPACK decoder keeps a dynamic table. The `settings` list sets the
advertised `QPACK_MAX_TABLE_CAPACITY` and `QPACK_BLOCKED_STREAMS`; a profile
without a `settings` list uses `qpack_max_table_capacity`,
`qpack_blocked_streams`, and `max_field_section_size`. The decoder applies
the peer's encoder stream, holds a field section that waits for inserts (up
to the advertised blocked streams), and sends Section Acknowledgment, Insert
Count Increment, and Stream Cancellation. The encoder does not use the
dynamic table.

## Proxies

HTTP/3 goes through a `socks5://` or `socks5h://` proxy with the `socks`
feature, which relays the QUIC datagrams with SOCKS5 `UDP ASSOCIATE`. An
`http://` or `https://` proxy cannot carry QUIC: under `Race` such a request
takes the `Auto` path, and `Http3` with such a proxy fails with
`Kind::Config`. When the cause is a missing `socks` feature, the message says
so. See [Proxies](proxies.md).

## Next

Read [TLS trust](tls-trust.md).
