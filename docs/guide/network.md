# Network

This page covers the layer under TLS: how a host name becomes an address, how
Leyline picks between IPv4 and IPv6, and which socket options it sets.

## Map a host to an address

`DnsConfig::resolve_host` sends one host to a fixed list of addresses without
a DNS lookup. Leyline tries them in order, as curl `--resolve` does. The host
name still goes into SNI and the `Host` header, so the server sees a normal
request.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .dns(leyline::DnsConfig::new().resolve_host(
        "example.com",
        ["203.0.113.10:443".parse().unwrap()],
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
            "203.0.113.10:443".parse().unwrap(),
            "203.0.113.11:443".parse().unwrap(),
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

When a host has IPv6 and IPv4 addresses, Leyline interleaves the two families
and starts the next attempt if the current one has not connected in time. The
first connection to complete wins.

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
| `interface` | unset | Interface name (accepted, not applied yet) |
| `strict` | `false` | Fail when the platform does not support an option |
| `happy_eyeballs` | enabled | Happy Eyeballs settings, or `None` to turn it off |

The timing and size setters take a value or `None`. Pass `None` to clear a
default, for example `tcp_keepalive(None)` to turn keepalive off.

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

Leyline applies `tcp_user_timeout` on Linux and Android. Other systems have no
equivalent socket option. Leyline does not apply `interface` on any system
yet; bind a source IP with `local_address` instead.

Where an option is unsupported, Leyline logs one warning for that option and
continues. With `strict(true)`, the connect fails with an
`io::ErrorKind::Unsupported` error. `tcp_keepalive_retries` falls back to the
operating system default on systems that cannot set it.

## TCP fingerprint

`SocketConfig` is separate from `TcpProfile`. `TcpProfile` sets the values
that make up the JA4T fingerprint and comes from the browser profile. Change
it only when you need a different TCP fingerprint. See
[Fingerprints](fingerprints.md).

## Next

Read [Fingerprints](fingerprints.md).
