# HTTP/3

The `http3` feature is on by default. It adds the `Http3` and `Race`
protocol policies and the QUIC stack. Build with `default-features = false`
to drop the QUIC stack. This page covers when a request uses HTTP/3, the
QUIC fingerprint keys of a profile, and HTTP/3 through a proxy.

## Protocol policy

`SessionBuilder::protocol` sets the session default, and
`RequestBuilder::protocol` overrides it for a single request while reusing the
session's connection pool.

| Variant | Behavior |
| --- | --- |
| `Auto` | HTTP/1.1 for `http://`. For `https://`, one TLS handshake that offers the profile's ALPN list; the server's choice decides HTTP/2 or HTTP/1.1 |
| `Http1` | Force HTTP/1.1 |
| `Http2` | Force HTTP/2 over TLS |
| `Http3` | Force HTTP/3 over QUIC. Needs the `http3` feature |
| `Race` | Race QUIC against TCP and TLS for origins known to speak HTTP/3. Needs the `http3` feature |

Without a `protocol` call, the default depends on the profile and build:

- `Race` when the `http3` feature is on, `unix_socket` is not set, and the profile sets `race = true` in `[h3]`. The bundled Chrome, Edge, Opera, Brave, Firefox, Safari macOS, and Safari iOS 18+ profiles do.
- `Auto` for every other session: plain `Session::new()`, Safari iOS 17, CFNetwork, OkHttp, `unix_socket` sessions, and builds without the `http3` feature.

Keep that default for a browser session: `Race` uses HTTP/3 where a browser
would and falls back to TCP. `Http3` forces QUIC on every request, the first
one included, and fails where QUIC is blocked. Use it only to test that an
origin speaks HTTP/3. `build()` fails with `Kind::Config` when `Http3` or
`Race` meets a profile with no `[h3]` table, such as the bare profile.

## The Alt-Svc gate

The first request to an origin never uses HTTP/3. As in Chrome, `Race` opens
no speculative QUIC connection: it races only an origin that advertised `h3`
in an `Alt-Svc` response header. The first request goes over HTTP/2 or
HTTP/1.1 and records the header; the next one can race.
`Response::version()` returns the `http::Version` that carried each response.

```rust,no_run
use leyline::{Browser, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::from(Browser::default());
let url = "https://www.cloudflare.com/";

let first = session.get(url).send().await?;
println!("first: {:?}", first.version());

let second = session.get(url).send().await?;
if second.version() == leyline::http::Version::HTTP_3 {
    println!("second: HTTP/3");
}
# Ok(())
# }
```

The second request can still use HTTP/2: the first transport to hand back a
connection carries the request, and a pooled HTTP/2 connection is often ready
first. The request is sent once. If HTTP/3 fails and TCP/TLS negotiated
HTTP/1.1, Leyline falls back to `Auto`; if both fail with errors, Leyline
returns the TCP/TLS error without dialing again. See `examples/http3.rs`.

A request is raced only when all of these hold; otherwise it takes the
`Auto` path:

- The scheme is `https` and the origin is known to speak HTTP/3.
- Neither the request body nor the response is streamed.
- `keep_alive` is `true` and `unix_socket` is not set (`Protocol::Http3` with `unix_socket` fails with `Kind::Config`).
- No proxy applies, or the proxy is `socks5://` or `socks5h://` and the `socks` feature (not a default feature) is enabled.

When an HTTP/3 connection to an origin fails, `Race` skips HTTP/3 for that
origin and proxy pair for 5 minutes. A successful HTTP/3 connection to the
pair ends the skip.

`Session::save` keeps the known origins, so a loaded session can race on its first request. See
[Accounts](accounts.md#the-connection-state).

### How Alt-Svc is read

Only an `h3=` entry for the same port counts, and its host must be empty or
equal to the origin's host. The origin is kept until the longest `ma` of the
matching entries, less the response's `Age`, ends; without `ma`, 24 hours.
Each `Alt-Svc` value replaces the one before it: `ma=0` on every matching
entry, `clear`, or a value with no usable `h3=` entry removes the origin. An
entry whose `ma` is not a number is ignored. The pool keeps at most 1024
origins and drops the one that expires first to make room.

## Body limits and cancellation

HTTP/3 enforces `max_body_size` (100 MiB default) on both buffered and
`.stream()` responses, failing with `Kind::BodyLimit`; pass
`.max_body_size(None)` to remove the cap. See [Responses](responses.md#bodies).

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

## Custom TOML & QUIC protocol internals

Skip this section unless you write your own profile TOML. Bundled profiles
already have an `[h3]` table.

An `[h3]` table must set these keys: `initial_max_data`,
`initial_max_stream_data_bidi_local`, `initial_max_stream_data_bidi_remote`,
`initial_max_stream_data_uni`, `initial_max_streams_bidi`,
`initial_max_streams_uni`, `max_idle_timeout_secs`, `max_udp_payload_size`,
`active_connection_id_limit`, and `dcid_length`. The keys below are optional.
`race = true` makes `Race` the session default.

| Key | Effect |
| --- | --- |
| `[h3.tls]` | Optional `[tls]` override for the QUIC ClientHello (supports `permute_extensions`, `extension_tail`, and `tls12_extensions`) |
| `dcid_length`, `scid_length` | Initial destination / source connection ID length (`dcid_length` accepts a fixed number or `{ weights = [[len, weight], ...] }`; `scid_length` defaults to `dcid_length`) |
| `transport_parameters` | Explicit transport parameter list (`{ id = N }`, `varint`, `hex`, `versions` for QUICv1/v2 `version_information`, or `{ grease = { id_bits, max_len } }`) |
| `transport_order` | `fixed`, `shuffle`, or `rotate` (`pinned = true` keeps an entry in place) |
| `initial_datagram_size` | First-flight Initial padding target (1200..`max_udp_payload_size`, max 1232 over IPv6; falls back to 1200 after loss) |
| `initial_crypto_split`, `initial_crypto_reorder` | Split Initial CRYPTO frames (`fill` or `even`) and optionally reorder at `sni_midpoint` |
| `max_ack_delay_ms` | Value sent for transport parameter 11 when listed |
| `settings`, `control_grease_frame` | Ordered HTTP/3 `SETTINGS` (`{ id, value }` or `{ grease = { id_bits, value_bits } }`) and optional trailing control GREASE frame |
| `pseudo_order`, `priority_update` | Pseudo-header order (default `method`, `scheme`, `authority`, `path`) and RFC 9218 `PRIORITY_UPDATE` framing |

Leyline speaks QUICv1 and QUICv2 (RFC 9369) with RFC 9368 compatible version
negotiation, accepts QUIC `DATAGRAM` frames when `max_datagram_frame_size` is
advertised, and handles `RESET_STREAM_AT` as `RESET_STREAM`.

### QPACK

Leyline decodes dynamic-table QPACK responses from servers (respecting
`QPACK_MAX_TABLE_CAPACITY`, `QPACK_BLOCKED_STREAMS`, and
`max_field_section_size` from `[h3]`) while sending static-only requests.

## Proxies

HTTP/3 runs through `socks5://` and `socks5h://` proxies when the `socks`
feature is enabled (via SOCKS5 `UDP ASSOCIATE`). `http://` and `https://`
proxies cannot carry QUIC: `Race` falls back to `Auto`, and `Http3` fails with
`Kind::Config`. See [HTTP/3 through a SOCKS5 proxy](proxies.md#http3-through-a-socks5-proxy).
