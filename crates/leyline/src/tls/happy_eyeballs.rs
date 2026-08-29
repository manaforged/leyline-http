//! Happy Eyeballs v2 (RFC 8305 §5) TCP connect racing.
//!
//! Once a [`Resolver`](crate::Resolver) has produced a list of
//! candidate addresses, [`happy_eyeballs_connect`] interleaves them
//! by address family (AAAA, A, AAAA, A, …), then launches TCP connect
//! attempts staggered by [`HappyEyeballsConfig::resolve_delay`]. The
//! first socket that completes its three-way handshake wins; any
//! in-flight attempts are aborted when their [`FuturesUnordered`]
//! task is dropped.
//!
//! This module only orchestrates the race. The caller (the
//! [`FingerprintConnector`](crate::tls::FingerprintConnector)) owns the
//! TCP profile, so the actual socket construction runs through a
//! closure it supplies — that keeps socket option tuning
//! (`IP_BIND_ADDRESS_NO_PORT`, initcwnd, `TCP_NODELAY`, …) in one
//! place.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::net::TcpStream;
use tokio::time::sleep;

/// Tunables for the Happy Eyeballs race.
#[derive(Debug, Clone, Copy)]
pub struct HappyEyeballsConfig {
    /// Gap between successive connect attempts. RFC 8305 §8
    /// recommends 250 ms and forbids anything below 10 ms.
    pub resolve_delay: Duration,
    /// Maximum number of parallel connect attempts. Caps worst-case
    /// fan-out on pathological resolvers that return dozens of
    /// addresses. Defaults to 8.
    pub attempt_limit: usize,
}

impl Default for HappyEyeballsConfig {
    fn default() -> Self {
        Self {
            resolve_delay: Duration::from_millis(250),
            attempt_limit: 8,
        }
    }
}

/// Reorder `addrs` so that v6 and v4 addresses alternate, v6 first.
///
/// Approximates RFC 6724 destination address selection by simply
/// interleaving — good enough for the "prefer IPv6, fall back to
/// IPv4" intent without depending on a DNS library that does full
/// prefix-match ranking.
pub(crate) fn interleave_by_family(addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let mut v6: Vec<SocketAddr> = Vec::new();
    let mut v4: Vec<SocketAddr> = Vec::new();
    for addr in addrs {
        if addr.is_ipv6() {
            v6.push(addr);
        } else {
            v4.push(addr);
        }
    }

    let total = v6.len() + v4.len();
    let mut out = Vec::with_capacity(total);
    let (mut i6, mut i4) = (0, 0);
    while i6 < v6.len() || i4 < v4.len() {
        if i6 < v6.len() {
            out.push(v6[i6]);
            i6 += 1;
        }
        if i4 < v4.len() {
            out.push(v4[i4]);
            i4 += 1;
        }
    }
    out
}

/// Race TCP connect attempts across `addrs`, staggered by
/// `config.resolve_delay`. Returns the first successful `(stream,
/// addr)` pair. If every attempt fails, returns the last error.
///
/// `connect_fn` builds a [`TcpStream`] for a single address — that
/// is where the caller applies its TCP fingerprint (socket2 options,
/// non-blocking mode, TFO, …) before awaiting completion.
pub(crate) async fn happy_eyeballs_connect<F, Fut>(
    addrs: Vec<SocketAddr>,
    config: HappyEyeballsConfig,
    connect_fn: F,
) -> Result<(TcpStream, SocketAddr), io::Error>
where
    F: Fn(SocketAddr) -> Fut + Send + Sync,
    Fut: Future<Output = Result<TcpStream, io::Error>> + Send + 'static,
{
    if addrs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no addresses resolved",
        ));
    }

    let interleaved = interleave_by_family(addrs);
    let attempt_limit = config.attempt_limit.max(1);
    let mut iter = interleaved.into_iter().take(attempt_limit);

    // One boxed future per in-flight attempt. `FuturesUnordered`
    // drops the remaining futures when we `return` out of the race,
    // aborting any stragglers.
    type Attempt = Pin<Box<dyn Future<Output = Result<(TcpStream, SocketAddr), io::Error>> + Send>>;
    let mut in_flight: FuturesUnordered<Attempt> = FuturesUnordered::new();
    let mut last_err: Option<io::Error> = None;

    // Kick off the first attempt immediately (RFC 8305 §5 step 2).
    if let Some(first) = iter.next() {
        let fut = connect_fn(first);
        in_flight.push(Box::pin(async move { fut.await.map(|s| (s, first)) }));
    }

    'outer: loop {
        match iter.next() {
            Some(addr) => {
                // Stagger: race the in-flight set against a
                // `resolve_delay` timer. If an attempt wins during the
                // stagger, return it. If they all fail during the
                // stagger, launch immediately. If the timer fires
                // first, launch the next attempt and loop.
                let sleep_fut = sleep(config.resolve_delay);
                tokio::pin!(sleep_fut);
                loop {
                    tokio::select! {
                        biased;
                        done = in_flight.next(), if !in_flight.is_empty() => {
                            match done {
                                Some(Ok(winner)) => return Ok(winner),
                                Some(Err(e)) => {
                                    last_err = Some(e);
                                    // Keep waiting on the timer unless we've
                                    // drained in-flight entirely — in which
                                    // case, start the next attempt right away.
                                    if in_flight.is_empty() {
                                        let fut = connect_fn(addr);
                                        in_flight.push(Box::pin(async move {
                                            fut.await.map(|s| (s, addr))
                                        }));
                                        continue 'outer;
                                    }
                                }
                                None => unreachable!(),
                            }
                        }
                        _ = &mut sleep_fut => {
                            let fut = connect_fn(addr);
                            in_flight.push(Box::pin(async move {
                                fut.await.map(|s| (s, addr))
                            }));
                            continue 'outer;
                        }
                    }
                }
            }
            None => {
                // All candidates launched — drain the in-flight set.
                if in_flight.is_empty() {
                    return Err(last_err.unwrap_or_else(|| {
                        io::Error::new(io::ErrorKind::NotFound, "no addresses resolved")
                    }));
                }
                while let Some(done) = in_flight.next().await {
                    match done {
                        Ok(winner) => return Ok(winner),
                        Err(e) => last_err = Some(e),
                    }
                }
                return Err(last_err.unwrap_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "no addresses resolved")
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests;
