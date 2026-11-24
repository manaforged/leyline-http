# Network

This page covers the layer under TLS: how a host name becomes an address, how
Leyline picks between IPv4 and IPv6, which socket options it sets, and how the
connection pool and the HSTS store work.

## Map a host to an address

`SessionBuilder::resolve` sends one host to a fixed list of addresses without
a DNS lookup. Leyline replaces the port of each address with the port of the
request URL, so the port you give is a placeholder. The host name still goes
into SNI and the `Host` header, so the server sees a normal request.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .resolve(
        "api.example.com",
        [
            "203.0.113.10:0".parse().unwrap(),
            "203.0.113.11:0".parse().unwrap(),
        ],
    )
    .build()?;
# let _ = session;
# Ok(())
# }
```

Hosts without an override go to the resolver. Call `resolve` once per host. A
later call for the same host replaces its list, and an empty list removes the
override.

## Replace the resolver

The default is the system resolver. Overrides from `resolve` always apply first.

- **DNS-over-HTTPS (RFC 8484):** Pass an `https://` endpoint with a host to `SessionBuilder::doh` (invalid or non-`https` URLs fail `build()` with `Kind::Config`). Built-in caches hold up to 256 IP entries and 256 ECH `HTTPS`/SVCB entries (TTL clamped to 1 s–3600 s; 60 s default/negative ECH TTL). DoH inherits the session's proxy, CA roots, `resolve` bootstrap overrides, connect timeout, and socket options (except `unix_socket`).
- **Custom resolver:** Implement `Resolver` (including optional `Resolver::resolve_ech` for ECH) and pass an `Arc<dyn Resolver>` to `SessionBuilder::dns_resolver`. Setting `dns_resolver` clears any `doh` endpoint.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .doh("https://cloudflare-dns.com/dns-query")
    .resolve("cloudflare-dns.com", ["1.1.1.1:443".parse().unwrap()])
    .build()?;
# let _ = session;
# Ok(())
# }
```

## Happy Eyeballs

TCP connections race resolved addresses (including `resolve` overrides):

- **Family order and rotation:** Interleaves IPv6 first, then alternates families. Each new connection rotates one address further per family to spread traffic evenly.
- **Staggered racing:** Starts one attempt, then starts the next after `happy_eyeballs_delay` (or immediately if all running attempts fail); the first connection to complete wins.
- **Outlier ejection:** An address that accumulates 5 consecutive failures (including slow in-flight attempts cancelled after `happy_eyeballs_delay` when another address wins) moves to the back of the order for 30 s, adding 30 s per repeat ejection up to 5 min. A successful connect resets its record; ejected addresses remain fallbacks if every other address fails.

A direct HTTP/3 connection uses one address: the first IPv4 address, or the
first address when the host has none.

| `SessionBuilder` setter | Default | Meaning |
| --- | --- | --- |
| `happy_eyeballs_delay` | 250 ms | Wait before Leyline starts the next address |
| `happy_eyeballs_attempts` | 8 | Most addresses tried for one connect |

```rust,no_run
# fn run() -> leyline::Result<()> {
use std::time::Duration;

let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .happy_eyeballs_delay(Duration::from_millis(100))
    .happy_eyeballs_attempts(4)
    .build()?;
# let _ = session;
# Ok(())
# }
```

Happy Eyeballs cannot be turned off; `happy_eyeballs_attempts(1)` tries one
address only.

## Socket options

These `SessionBuilder` setters set options on each TCP socket before it
connects.

| Method | Default | Effect |
| --- | --- | --- |
| `tcp_nodelay` | unset | `TCP_NODELAY` |
| `tcp_keepalive` | 60 s | Idle time before the first keepalive probe |
| `tcp_keepalive_interval` | 30 s | Time between probes |
| `tcp_keepalive_retries` | 3 | Failed probes before the socket closes |
| `tcp_user_timeout` | unset | `TCP_USER_TIMEOUT`: how long sent data may stay unacknowledged (Linux, Android) |
| `send_buffer_size`, `recv_buffer_size` | unset | `SO_SNDBUF`, `SO_RCVBUF` |
| `local_address` | unset | Source IP to bind for that IP's address family; call it once per family |
| `interface` | unset | Bind sockets to a network interface (`SO_BINDTODEVICE` on Linux, `IP_BOUND_IF` on Apple) |
| `unix_socket` | unset | Route all HTTP and HTTPS traffic over a Unix domain socket path (Unix) |
| `strict_socket_options` | `false` | Fail when the platform does not support an option |

The timing and size setters take a value or `None`, which clears it. Leyline
sets TCP keepalive when any of its three values is set; to turn it off, pass
`None` to all three.

```rust,no_run
# fn run() -> leyline::Result<()> {
use std::time::Duration;

let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .tcp_nodelay(true)
    .tcp_user_timeout(Duration::from_secs(20))
    .tcp_keepalive(Duration::from_secs(30))
    .tcp_keepalive_interval(Duration::from_secs(10))
    .build()?;
# let _ = session;
# Ok(())
# }
```

Set `tcp_user_timeout` when a pooled connection can die without a FIN or RST,
for example when a proxy or NAT drops its state. Without it, the kernel
retransmits for many minutes and the request waits for its total timeout.
With it, the kernel closes the socket, the request fails, and a retry policy
can use a fresh connection.

Where an option is unsupported, Leyline logs one warning on `leyline::socket`
and continues. With `strict_socket_options(true)`, connect fails with
`Kind::Config` wrapping `io::ErrorKind::Unsupported`. `tcp_keepalive_retries`
falls back to the OS default where it cannot be set.

### Local binding and Unix sockets

- **`local_address(ip)`**: Call once for IPv4 and/or once for IPv6; pass `None` to clear both families.
- **`interface(name)`**: Binds outgoing sockets to an interface (`SO_BINDTODEVICE` on Linux/Android/Fuchsia, `IP_BOUND_IF` on Apple). Empty names or NUL bytes fail `build()` with `Kind::Config`; unknown or unsupported interfaces fail connect with `Kind::Config` even when `strict_socket_options(false)`.
- **`unix_socket(path)`** (Unix only): Routes both `http://` and `https://` traffic over a local Unix domain socket (TLS SNI and `Host` still come from the request URL). Disables environment proxies; combining it with any proxy or `Protocol::Http3` fails with `Kind::Config`, and `Protocol::Race` skips HTTP/3.

These socket options are separate from `leyline::profile::TcpProfile`
(`TcpProfile::from_ja4t` or TOML `FromStr`), which shapes the JA4T TCP
fingerprint. See [Fingerprints](fingerprints.md#customize-tcpprofile).

## Connection pool

Each session has a connection pool. These `SessionBuilder` setters set its
limits, and every limit applies to one pool.

| Setter | Default | Effect |
| --- | --- | --- |
| `max_connections` | 2048 | Pool entries, one per host, port, scheme, proxy, and transport. Set it above the number of hosts you talk to. At the limit, a new entry replaces the least recently used entry in its shard |
| `max_connections_per_host` | 256 | Max connections per host/port/scheme/proxy/transport. Applies to HTTP/1.1 (waiting requests are bounded by `HostLimits::max_pending`, 1024 default, failing with `Kind::Overloaded`) and to bare-session HTTP/2 when open connections hit the server's stream cap. Browser-profile sessions stay on 1 HTTP/2 connection per origin unless you set this explicitly; HTTP/3 uses 1 connection per origin |
| `pool_idle_timeout` | 300 s | An idle connection closes after this time |
| `pool_max_lifetime` | no limit | A connection is not reused after this time. In-flight requests finish. Set it so traffic follows a DNS or load balancer change |
| `keep_alive` | `true` | Keep connections open for reuse |
| `http2_ping_after_idle` | 10 s | Sends an HTTP/2 `PING` after this idle period or after a header timeout (`None` disables both). Idempotent replayable requests are sent alongside the `PING` without waiting a round trip for the `ACK` |
| `http2_ping_timeout` | 2 s | The wait for the answer to that `PING` |

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .max_connections(512)
    .max_connections_per_host(8)
    .build()?;
# let _ = session;
# Ok(())
# }
```

Clones and every derived session share the pool, except one built after a
connection setting changed (the [list](sessions.md#derive-a-session)), or `to_builder().share_connections(false)`, which
starts a new, empty one. A session with another identity keeps its connections in a
partition of its own, inside the shared pool and its limits. See
[Sessions](sessions.md#derive-a-session).

When every stream slot of an HTTP/2 connection is in use, one request waits
inside the connection and later requests wait in a queue of at most 1024. The
request's timeout covers the wait, and a request dropped while it waits is
never sent.

### One handshake per connection

An HTTPS connection always sends the profile's ClientHello, with its ALPN
list, for example `h2, http/1.1`, and its ALPS extension. The server's ALPN
choice decides HTTP/2 or HTTP/1.1 on that same connection. Leyline never
dials again to change the protocol, so a streaming request body to an
HTTP/1.1-only origin works on the first request.

When an origin negotiates HTTP/1.1, the pool remembers it for 10 minutes, for
at most 1024 origins. The next connection still sends the full ClientHello,
and if the origin then answers `h2`, Leyline uses HTTP/2 and forgets the
entry.

`Protocol::Http2` sends every request over HTTP/2 with the same
ClientHello; when the server picks another protocol, the request fails with
`Kind::Http2`. `Protocol::Http1` and a WebSocket over HTTP/1.1 offer
`http/1.1` only.

## Limit requests per host

`HostLimits` limits the requests to each origin, apart from the pool limits,
which count connections. See [Limit each host](crawling.md#limit-each-host).

## HSTS

Leyline keeps an HSTS store per session (RFC 6797). It records the first
`Strict-Transport-Security` header of each `https` response. The header needs
`max-age`; `includeSubDomains` also covers the subdomains, and `max-age=0`
removes the host. IP addresses are ignored. A session with `danger_accept_invalid_certs(true)` or
`danger_accept_invalid_hostnames(true)` records nothing, because it cannot
trust the header.

A later `http://` request or redirect to a recorded host, or a covered
subdomain, goes to `https` instead, and an explicit port 80 becomes 443.
WebSocket requests are not upgraded.

Derived sessions, those with another identity included, share the store;
only one with a new pool starts with an empty one. `Session::save` keeps the store. See
[Accounts](accounts.md#the-connection-state).

## Next

Read [Logging and tracing](logging.md). Timeouts are covered in
[Retries and timeouts](retries-and-timeouts.md#the-timeouts).
