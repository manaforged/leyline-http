# Network

This page covers the layer under TLS: how a host name becomes an address, how
Leyline picks between IPv4 and IPv6, and which socket options it sets.

## Map a host to an address

`DnsConfig::resolve_host` sends one host to a fixed list of addresses without
a DNS lookup. Leyline replaces the port of each address with the port of the
request URL, so the port you give is a placeholder. The order of the attempts
follows [Happy Eyeballs](#happy-eyeballs): IPv6 first, then alternating
families. The host name goes into SNI and the `Host` header, so the server
sees a normal request.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .dns(leyline::DnsConfig::new().resolve_host(
        "example.com",
        ["203.0.113.10:0".parse().unwrap()],
    ))
    .build()?;
# let _ = session;
# Ok(())
# }
```

Hosts without an override go to the resolver. A later call for the same host
replaces its list. An empty list removes the override.

## Replace the resolver

The default is `tls::SystemResolver`. To use your own, implement
`tls::Resolver` and pass it to `SessionBuilder::dns` as an
`Arc<dyn Resolver>`.

`DnsConfig` holds a resolver and the overrides as one value, so several
builders can share it. Put every override in one `DnsConfig`: a second `dns`
call replaces the first.

```rust,no_run
# fn run() -> leyline::Result<()> {
use leyline::DnsConfig;

let dns = DnsConfig::new()
    .resolve_host(
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

## Happy Eyeballs

Leyline connects to the addresses of a host in this order: IPv6 first, then
alternating between the two families. Inside one family, the order stays as
the resolver or `resolve_host` gave it. Leyline starts one attempt. It starts
the next when `resolve_delay` passes with no connection, or at once when every
running attempt has failed. Attempts in progress keep running, and the first
connection to complete wins. Leyline tries at most `attempt_limit`
addresses for one connect.

This applies to every TCP connection Leyline opens, including a host that you
map with `resolve_host`. A direct HTTP/3 connection uses one address: the first
IPv4 address, or the first address when the host has no IPv4 address.

`tls::HappyEyeballsConfig` has two setters. Pass it to
`SocketConfig::happy_eyeballs`:

| Setting | Default | Meaning |
|---|---|---|
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

`happy_eyeballs(None)` keeps the defaults. Nothing turns Happy Eyeballs off.
To make Leyline try one address only, set `attempt_limit(1)`.

## Socket options

`SocketConfig` sets options on each TCP socket before it connects.

| Method | Default | Effect |
|---|---|---|
| `tcp_nodelay` | unset | `TCP_NODELAY` |
| `tcp_keepalive` | 60 s | Idle time before the first keepalive probe |
| `tcp_keepalive_interval` | 30 s | Time between probes |
| `tcp_keepalive_retries` | 3 | Failed probes before the socket closes |
| `tcp_user_timeout` | unset | `TCP_USER_TIMEOUT`: how long sent data may stay unacknowledged (Linux, Android) |
| `send_buffer_size`, `recv_buffer_size` | unset | `SO_SNDBUF`, `SO_RCVBUF` |
| `local_address` | unset | Source IP to bind |
| `local_ipv4`, `local_ipv6` | unset | Source IP to bind for that address family |
| `strict` | `false` | Fail when the platform does not support an option |
| `happy_eyeballs` | 250 ms delay, 8 attempts | Happy Eyeballs settings. `None` keeps the defaults |

The timing and size setters take a value or `None`. `None` clears the value.
Leyline sets TCP keepalive when any of its three values is set. To turn
keepalive off, pass `None` to `tcp_keepalive`, `tcp_keepalive_interval`, and
`tcp_keepalive_retries`.

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

`socket` replaces the whole value. Start from `SocketConfig::new()`,
which carries the defaults, and change only what you need.

### Why set `tcp_user_timeout`

A pooled connection can die without a FIN or RST, for example when a proxy or
NAT drops its state. Without `TCP_USER_TIMEOUT` the kernel keeps
retransmitting for many minutes, and the request waits until your total
timeout expires. With it, the kernel closes the socket after the given time,
the request fails, and a retry policy can use a fresh connection.

### Unsupported options

Leyline applies `tcp_user_timeout` on Linux and Android only.

Where an option is unsupported, Leyline logs one warning for that option and
continues. With `strict(true)`, the connect fails with an
`io::ErrorKind::Unsupported` error. `tcp_keepalive_retries` falls back to the
operating system default on systems that cannot set it.

## TCP fingerprint

`SocketConfig` is separate from `TcpProfile`. `TcpProfile` sets the values
that make up the JA4T fingerprint. It comes from the platform of the session,
unless you pass your own to `SessionBuilder::tcp_profile`. Change it only when
you need a different TCP fingerprint. See [Fingerprints](fingerprints.md).

## Next

Read [Logging and tracing](logging.md).
