use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::{Error, HttpVersion};

#[non_exhaustive]
pub struct Dns<'a> {
    pub id: u64,
    pub host: &'a str,
    pub port: u16,
    pub addrs: usize,
    pub elapsed: Duration,
}

#[non_exhaustive]
pub struct Connect<'a> {
    pub id: u64,
    pub host: &'a str,
    pub port: u16,
    pub reused: bool,
    pub elapsed: Duration,
}

#[non_exhaustive]
pub struct Tls<'a> {
    pub id: u64,
    pub host: &'a str,
    pub version: Option<&'a str>,
    pub cipher: Option<&'a str>,
    pub alpn: Option<&'a str>,
    pub elapsed: Duration,
}

#[non_exhaustive]
pub struct Sent<'a> {
    pub id: u64,
    pub host: &'a str,
    pub method: &'a str,
    pub path: &'a str,
    pub protocol: HttpVersion,
    pub elapsed: Duration,
}

#[non_exhaustive]
pub struct Head<'a> {
    pub id: u64,
    pub host: &'a str,
    pub status: u16,
    pub protocol: HttpVersion,
    pub elapsed: Duration,
    pub headers: &'a http::HeaderMap,
}

#[non_exhaustive]
pub struct Done<'a> {
    pub id: u64,
    pub elapsed: Duration,
    pub outcome: Result<(), &'a Error>,
}

#[expect(
    unused_variables,
    reason = "no-op defaults keep the event name visible in the rendered signature"
)]
pub trait Trace: Send + Sync + 'static {
    fn dns(&self, ev: &Dns<'_>) {}
    fn connect(&self, ev: &Connect<'_>) {}
    fn tls(&self, ev: &Tls<'_>) {}
    fn sent(&self, ev: &Sent<'_>) {}
    fn head(&self, ev: &Head<'_>) {}
    fn done(&self, ev: &Done<'_>) {}
}

#[derive(Clone)]
pub(crate) struct Ctx {
    hook: Arc<dyn Trace>,
    id: u64,
    start: Instant,
}

tokio::task_local! {
    static CURRENT: Ctx;
}

static NEXT: AtomicU64 = AtomicU64::new(1);

pub(crate) async fn scope<F>(hook: Option<&Arc<dyn Trace>>, fut: F) -> F::Output
where
    F: Future,
{
    match hook {
        Some(hook) => {
            let ctx = Ctx {
                hook: Arc::clone(hook),
                id: NEXT.fetch_add(1, Ordering::Relaxed),
                start: Instant::now(),
            };
            CURRENT.scope(ctx, fut).await
        }
        None => fut.await,
    }
}

pub(crate) fn carry<F>(fut: F) -> impl Future<Output = F::Output> + Send
where
    F: Future + Send,
{
    let ctx = CURRENT.try_with(Clone::clone).ok();
    async move {
        match ctx {
            Some(ctx) => CURRENT.scope(ctx, fut).await,
            None => fut.await,
        }
    }
}

pub(crate) fn on() -> bool {
    CURRENT.try_with(|_| ()).is_ok()
}

fn with(emit: impl FnOnce(&Ctx)) {
    CURRENT.try_with(emit).unwrap_or(())
}

pub(crate) fn dns(host: &str, port: u16, addrs: usize, elapsed: Duration) {
    with(|ctx| {
        ctx.hook.dns(&Dns {
            id: ctx.id,
            host,
            port,
            addrs,
            elapsed,
        });
    });
}

pub(crate) fn connect(host: &str, port: u16, reused: bool, elapsed: Duration) {
    with(|ctx| {
        ctx.hook.connect(&Connect {
            id: ctx.id,
            host,
            port,
            reused,
            elapsed,
        });
    });
}

pub(crate) fn tls(
    host: &str,
    version: Option<&str>,
    cipher: Option<&str>,
    alpn: Option<&str>,
    elapsed: Duration,
) {
    with(|ctx| {
        ctx.hook.tls(&Tls {
            id: ctx.id,
            host,
            version,
            cipher,
            alpn,
            elapsed,
        });
    });
}

pub(crate) fn sent(host: &str, method: &str, path: &str, protocol: HttpVersion, elapsed: Duration) {
    with(|ctx| {
        ctx.hook.sent(&Sent {
            id: ctx.id,
            host,
            method,
            path,
            protocol,
            elapsed,
        });
    });
}

pub(crate) fn head<I, N, V>(
    host: &str,
    status: u16,
    protocol: HttpVersion,
    elapsed: Duration,
    headers: I,
) where
    I: IntoIterator<Item = (N, V)>,
    N: AsRef<[u8]>,
    V: Into<bytes::Bytes>,
{
    with(|ctx| {
        let headers = crate::core::header_map(headers);
        ctx.hook.head(&Head {
            id: ctx.id,
            host,
            status,
            protocol,
            elapsed,
            headers: &headers,
        });
    });
}

pub(crate) fn done(outcome: Result<(), &Error>) {
    with(|ctx| {
        ctx.hook.done(&Done {
            id: ctx.id,
            elapsed: ctx.start.elapsed(),
            outcome,
        });
    });
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TracingTrace;

impl Trace for TracingTrace {
    fn dns(&self, ev: &Dns<'_>) {
        tracing::debug!(target: "leyline::trace", id = ev.id, host = ev.host, port = ev.port, addrs = ev.addrs, elapsed_ms = ms(ev.elapsed), "dns");
    }

    fn connect(&self, ev: &Connect<'_>) {
        tracing::debug!(target: "leyline::trace", id = ev.id, host = ev.host, port = ev.port, reused = ev.reused, elapsed_ms = ms(ev.elapsed), "connect");
    }

    fn tls(&self, ev: &Tls<'_>) {
        tracing::debug!(target: "leyline::trace", id = ev.id, host = ev.host, version = ev.version, cipher = ev.cipher, alpn = ev.alpn, elapsed_ms = ms(ev.elapsed), "tls");
    }

    fn sent(&self, ev: &Sent<'_>) {
        tracing::debug!(target: "leyline::trace", id = ev.id, host = ev.host, method = ev.method, path = ev.path, protocol = ?ev.protocol, elapsed_ms = ms(ev.elapsed), "sent");
    }

    fn head(&self, ev: &Head<'_>) {
        tracing::debug!(target: "leyline::trace", id = ev.id, host = ev.host, status = ev.status, protocol = ?ev.protocol, elapsed_ms = ms(ev.elapsed), "head");
    }

    fn done(&self, ev: &Done<'_>) {
        match ev.outcome {
            Ok(()) => {
                tracing::debug!(target: "leyline::trace", id = ev.id, elapsed_ms = ms(ev.elapsed), "done")
            }
            Err(e) => {
                tracing::debug!(target: "leyline::trace", id = ev.id, elapsed_ms = ms(ev.elapsed), kind = ?e.kind(), error = %e, "done")
            }
        }
    }
}

fn ms(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}

impl<T: Trace> Trace for Arc<T> {
    fn dns(&self, ev: &Dns<'_>) {
        T::dns(self, ev);
    }

    fn connect(&self, ev: &Connect<'_>) {
        T::connect(self, ev);
    }

    fn tls(&self, ev: &Tls<'_>) {
        T::tls(self, ev);
    }

    fn sent(&self, ev: &Sent<'_>) {
        T::sent(self, ev);
    }

    fn head(&self, ev: &Head<'_>) {
        T::head(self, ev);
    }

    fn done(&self, ev: &Done<'_>) {
        T::done(self, ev);
    }
}

#[cfg(test)]
mod tests;
