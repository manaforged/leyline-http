# Network

This page covers the layer under TLS: how a host name becomes an address, how
Leyline picks between IPv4 and IPv6, which socket options it sets, and how the
connection pool and the HSTS store work.

## Map a host to an address

`DnsConfig::resolve_host` sends one host to a fixed list of addresses without
a DNS lookup. Leyline replaces the port of each address with the port of the
request URL, so the port you give is a placeholder. The host name still goes
into SNI and the `Host` header, so the server sees a normal request.

```rust,no_run
# fn run() -> leyline::Result<()> {
use leyline::DnsConfig;

let dns = DnsConfig::new().resolve_host(
    "api.example.com",
    [
        "203.0.113.10:0".parse().unwrap(),
        "203.0.113.11:0".parse().unwrap(),
    ],
);
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .dns(dns)
    .build()?;
# let _ = session;
# Ok(())
# }
```

Hosts without an override go to the resolver. A later call for the same host
replaces its list, and an empty list removes the override. Put every override
in one `DnsConfig`: a second `dns` call replaces the first.

## Replace the resolver

The default is `tls::SystemResolver`. To use your own, implement
`tls::Resolver` and pass it to `SessionBuilder::dns` as an
`Arc<dyn Resolver>`. `DnsConfig` holds the resolver and the overrides as one
value, so several builders can share it.

## Happy Eyeballs

Leyline connects to the addresses of a host IPv6 first, then alternating
between the families. Inside one family, the order stays as the resolver or
`resolve_host` gave it. Leyline starts one attempt, and starts the next when
`resolve_delay` passes with no connection, or at once when every running
attempt has failed. The first connection to complete wins.

This applies to every TCP connection, mapped hosts included. A direct HTTP/3
connection uses one address: the first IPv4 address, or the first address
when the host has none.

| `tls::HappyEyeballsConfig` | Default | Meaning |
| --- | --- | --- |
| `resolve_delay` | 250 ms | Wait before Leyline starts the next address |
| `attempt_limit` | 8 | Most addresses tried for one connect |

```rust,no_run
# fn run() -> leyline::Result<()> {
use std::time::Duration;
use leyline::SocketConfig;
use leyline::tls::HappyEyeballsConfig;

let he = HappyEyeballsConfig::new()
    .resolve_delay(Duration::from_millis(100))
    .attempt_limit(4);
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .socket(SocketConfig::new().happy_eyeballs(he))
    .build()?;
# let _ = session;
# Ok(())
# }
```

Happy Eyeballs cannot be turned off; `attempt_limit(1)` tries one address
only. `happy_eyeballs(None)` keeps the defaults.

## Socket options

`SocketConfig` sets options on each TCP socket before it connects.
`SessionBuilder::socket` replaces the whole value, so start from
`SocketConfig::new()`, which carries the defaults.

| Method | Default | Effect |
| --- | --- | --- |
| `tcp_nodelay` | unset | `TCP_NODELAY` |
| `tcp_keepalive` | 60 s | Idle time before the first keepalive probe |
| `tcp_keepalive_interval` | 30 s | Time between probes |
| `tcp_keepalive_retries` | 3 | Failed probes before the socket closes |
| `tcp_user_timeout` | unset | `TCP_USER_TIMEOUT`: how long sent data may stay unacknowledged (Linux, Android) |
| `send_buffer_size`, `recv_buffer_size` | unset | `SO_SNDBUF`, `SO_RCVBUF` |
| `local_address` | unset | Source IP to bind |
| `local_ipv4`, `local_ipv6` | unset | Source IP to bind for that address family |
| `strict` | `false` | Fail when the platform does not support an option |
| `happy_eyeballs` | see above | [Happy Eyeballs](#happy-eyeballs) settings |

The timing and size setters take a value or `None`, which clears it. Leyline
sets TCP keepalive when any of its three values is set; to turn it off, pass
`None` to all three.

```rust,no_run
# fn run() -> leyline::Result<()> {
use std::time::Duration;
use leyline::SocketConfig;

let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .socket(
        SocketConfig::new()
            .tcp_nodelay(true)
            .tcp_user_timeout(Duration::from_secs(20))
            .tcp_keepalive(Duration::from_secs(30))
            .tcp_keepalive_interval(Duration::from_secs(10)),
    )
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

Where an option is unsupported, Leyline logs one warning for it on the
`leyline::socket` target and continues. With `strict(true)`, the connect fails
with an `io::ErrorKind::Unsupported` error. `tcp_keepalive_retries` falls back
to the operating system default where it cannot be set.

`SocketConfig` is separate from `TcpProfile`, which sets the values of the
JA4T fingerprint and comes from the session platform. See
[Fingerprints](fingerprints.md).

## Connection pool

Each session has a connection pool. `PoolConfig` sets its limits, and every
limit applies to one pool.

| Setting | Default | Effect |
| --- | --- | --- |
| `max_connections` | 2048 | Pool entries. The least recently used entry leaves first |
| `max_h1_conns_per_host` | 256 | HTTP/1.1 connections per host, port, scheme, proxy, and transport. A request over the limit waits for a free slot |
| `idle_timeout` | 300 s | An idle connection closes after this time |
| `keepalive` | `true` | Keep connections open for reuse |
| `h2_ping_after_idle` | 10 s | An idle HTTP/2 connection gets a `PING` after this time. `None` turns it off |
| `h2_ping_timeout` | 2 s | The wait for the answer to that `PING` |

```rust,no_run
use leyline::PoolConfig;

# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .pool(PoolConfig::new().max_connections(512).max_h1_conns_per_host(8))
    .build()?;
# let _ = session;
# Ok(())
# }
```

Clones and every derived session share the pool, except `fresh_pool`, which
starts a new, empty one. A `with_identity` session keeps its connections in a
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

`ProtocolPolicy::Http2` sends every request over HTTP/2 with the same
ClientHello; when the server picks another protocol, the request fails with
`Kind::Http2`. `ProtocolPolicy::Http1` and a WebSocket over HTTP/1.1 offer
`http/1.1` only.

## Limit requests per host

`HostLimits` limits the requests to each origin, apart from the pool limits,
which count connections. See [Limit each host](crawling.md#limit-each-host).

## HSTS

Leyline keeps an HSTS store per session (RFC 6797). It records the first
`Strict-Transport-Security` header of each `https` response. The header needs
`max-age`; `includeSubDomains` also covers the subdomains, and `max-age=0`
removes the host. IP addresses are ignored.

A later `http://` request or redirect to a recorded host, or a covered
subdomain, goes to `https` instead, and an explicit port 80 becomes 443.
WebSocket requests are not upgraded.

Derived sessions, `with_identity` included, share the store; only
`fresh_pool` starts with an empty one. `Session::state()` saves the store,
and `SessionState::restore_into` loads it. See
[Accounts](accounts.md#keep-the-connection-state).

## Next

Read [Logging and tracing](logging.md). Timeouts are covered in
[Retries and timeouts](retries-and-timeouts.md#the-timeouts).
