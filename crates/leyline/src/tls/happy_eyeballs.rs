//! Happy Eyeballs v2 (RFC 8305 §5) TCP connect racing.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::net::TcpStream;
use tokio::time::sleep;

/// Tunables for the Happy Eyeballs race. Start from [`HappyEyeballsConfig::default`] and set one field per call.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct HappyEyeballsConfig {
    /// Gap between successive connect attempts.
    pub resolve_delay: Duration,
    /// Maximum number of parallel connect attempts.
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

impl HappyEyeballsConfig {
    /// Create default Happy Eyeballs tunables.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the gap between successive connect attempts.
    pub fn resolve_delay(mut self, d: Duration) -> Self {
        self.resolve_delay = d;
        self
    }

    /// Set the maximum number of parallel connect attempts.
    pub fn attempt_limit(mut self, n: usize) -> Self {
        self.attempt_limit = n;
        self
    }
}

/// Reorder `addrs` so that v6 and v4 addresses alternate, v6 first.
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

/// Race TCP connect attempts across `addrs`, staggered by `config.resolve_delay`.
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

    type Attempt = Pin<Box<dyn Future<Output = Result<(TcpStream, SocketAddr), io::Error>> + Send>>;
    let mut in_flight: FuturesUnordered<Attempt> = FuturesUnordered::new();
    let mut last_err: Option<io::Error> = None;

    if let Some(first) = iter.next() {
        let fut = connect_fn(first);
        in_flight.push(Box::pin(async move { fut.await.map(|s| (s, first)) }));
    }

    'outer: loop {
        match iter.next() {
            Some(addr) => {
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
