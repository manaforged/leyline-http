use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::net::TcpStream;
use tokio::time::sleep;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) struct HappyEyeballs {
    pub(crate) resolve_delay: Duration,
    pub(crate) attempt_limit: usize,
}

impl Default for HappyEyeballs {
    fn default() -> Self {
        Self {
            resolve_delay: Duration::from_millis(250),
            attempt_limit: 8,
        }
    }
}

pub(crate) fn interleave_by_family(addrs: Vec<SocketAddr>, turn: usize) -> Vec<SocketAddr> {
    let mut v6: Vec<SocketAddr> = Vec::new();
    let mut v4: Vec<SocketAddr> = Vec::new();
    for addr in addrs {
        if addr.is_ipv6() {
            v6.push(addr);
        } else {
            v4.push(addr);
        }
    }

    for family in [&mut v6, &mut v4] {
        if !family.is_empty() {
            let len = family.len();
            family.rotate_left(turn % len);
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

type Attempt = Pin<Box<dyn Future<Output = Result<(TcpStream, SocketAddr), io::Error>> + Send>>;

pub(crate) async fn happy_eyeballs_connect<F, Fut>(
    order: Vec<SocketAddr>,
    config: HappyEyeballs,
    connect_fn: F,
) -> Result<(TcpStream, SocketAddr), io::Error>
where
    F: Fn(SocketAddr) -> Fut + Send + Sync,
    Fut: Future<Output = Result<TcpStream, io::Error>> + Send + 'static,
{
    let mut queue = order.into_iter().take(config.attempt_limit.max(1));
    let launch = |addr: SocketAddr| -> Attempt {
        let fut = connect_fn(addr);
        Box::pin(async move { fut.await.map(|stream| (stream, addr)) })
    };
    let mut in_flight: FuturesUnordered<Attempt> = FuturesUnordered::new();
    let mut last_err = None;
    let Some(first) = queue.next() else {
        return Err(no_connection(last_err));
    };
    in_flight.push(launch(first));
    let mut waiting = queue.next();
    let timer = sleep(config.resolve_delay);
    tokio::pin!(timer);
    loop {
        tokio::select! {
            biased;
            done = in_flight.next(), if !in_flight.is_empty() => match done {
                Some(Ok(winner)) => return Ok(winner),
                Some(Err(error)) => last_err = Some(error),
                None => unreachable!(),
            },
            () = &mut timer, if waiting.is_some() => {}
        }
        let next_due = timer.is_elapsed() || in_flight.is_empty();
        if !next_due {
            continue;
        }
        let Some(addr) = waiting.take() else {
            if in_flight.is_empty() {
                return Err(no_connection(last_err));
            }
            continue;
        };
        in_flight.push(launch(addr));
        waiting = queue.next();
        timer
            .as_mut()
            .reset(tokio::time::Instant::now() + config.resolve_delay);
    }
}

fn no_connection(last_err: Option<io::Error>) -> io::Error {
    last_err.unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no addresses resolved"))
}

#[cfg(test)]
mod tests;
