//! Per-request lifecycle hooks: DNS, connect, TLS, send, response head, and completion.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::{Error, HttpVersion, ResponseTiming};

/// Name resolution finished for one request.
#[non_exhaustive]
pub struct Dns<'a> {
    /// Attempt this event belongs to.
    pub id: u64,
    /// Host name that was resolved.
    pub host: &'a str,
    /// Port the addresses were resolved for.
    pub port: u16,
    /// Number of addresses the resolver returned.
    pub addrs: usize,
    /// Time the resolver took.
    pub elapsed: Duration,
}

/// A connection became available for one request.
#[non_exhaustive]
pub struct Connect<'a> {
    /// Attempt this event belongs to.
    pub id: u64,
    /// Host the connection points at.
    pub host: &'a str,
    /// Port the connection points at.
    pub port: u16,
    /// `true` when the connection came from the pool; then `elapsed` is zero.
    pub reused: bool,
    /// Time the TCP or QUIC connect took.
    pub elapsed: Duration,
}

/// A TLS handshake completed for one request.
#[non_exhaustive]
pub struct Tls<'a> {
    /// Attempt this event belongs to.
    pub id: u64,
    /// Host the handshake authenticated.
    pub host: &'a str,
    /// Negotiated TLS version, as the library reports it.
    pub version: Option<&'a str>,
    /// Negotiated cipher suite.
    pub cipher: Option<&'a str>,
    /// Protocol ALPN selected.
    pub alpn: Option<&'a str>,
    /// Time the handshake took.
    pub elapsed: Duration,
}

/// The request was handed to the wire.
#[non_exhaustive]
pub struct Sent<'a> {
    /// Attempt this event belongs to.
    pub id: u64,
    /// Host the request went to.
    pub host: &'a str,
    /// Protocol that framed the request.
    pub protocol: HttpVersion,
    /// Time spent framing and writing the request.
    pub elapsed: Duration,
}

/// The response head (first byte) arrived.
#[non_exhaustive]
pub struct Head<'a> {
    /// Attempt this event belongs to.
    pub id: u64,
    /// Host the response came from.
    pub host: &'a str,
    /// Response status code.
    pub status: u16,
    /// Protocol that carried the response.
    pub protocol: HttpVersion,
    /// Time from the end of the request send to the response head.
    pub elapsed: Duration,
}

/// The attempt finished: the body is complete, or the request failed.
#[non_exhaustive]
pub struct Done<'a> {
    /// Attempt this event belongs to.
    pub id: u64,
    /// Time from the start of the attempt.
    pub elapsed: Duration,
    /// `Ok` when the response reached the caller, `Err` with the failure otherwise.
    pub outcome: Result<(), &'a Error>,
}

/// Observes one request's lifecycle; every method has a no-op default. Hooks run inline on the request task, so a slow listener slows the request. Do not block, lock, or await inside one.
#[expect(
    unused_variables,
    reason = "no-op defaults keep the event name visible in the rendered signature"
)]
pub trait Trace: Send + Sync + 'static {
    /// Name resolution finished.
    fn dns(&self, ev: &Dns<'_>) {}
    /// A connection became available; `ev.reused` tells a pool hit from a fresh dial.
    fn connect(&self, ev: &Connect<'_>) {}
    /// A TLS handshake completed.
    fn tls(&self, ev: &Tls<'_>) {}
    /// The request reached the wire.
    fn sent(&self, ev: &Sent<'_>) {}
    /// The response head arrived.
    fn head(&self, ev: &Head<'_>) {}
    /// The attempt finished.
    fn done(&self, ev: &Done<'_>) {}
}

/// The listener and identity of the attempt running on this task.
#[derive(Clone)]
pub(crate) struct Ctx {
    hook: Arc<dyn Trace>,
    id: u64,
    start: Instant,
}

tokio::task_local! {
    static CURRENT: Ctx;
}

/// Monotonic attempt counter; every scope takes the next value.
static NEXT: AtomicU64 = AtomicU64::new(1);

/// Run `fut` as one traced attempt. Without a listener the future runs unchanged.
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

/// Carry the calling task's trace context into a spawned task. Call this in the parent task.
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

/// `true` when the current task carries a listener.
pub(crate) fn on() -> bool {
    CURRENT.try_with(|_| ()).is_ok()
}

/// Hand the current context to `emit`, or do nothing when the task carries no listener.
fn with(emit: impl FnOnce(&Ctx)) {
    CURRENT.try_with(emit).unwrap_or(())
}

/// Report finished name resolution.
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

/// Report an available connection.
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

/// Report a completed TLS handshake.
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

/// Report a request that reached the wire.
pub(crate) fn sent(host: &str, protocol: HttpVersion, elapsed: Duration) {
    with(|ctx| {
        ctx.hook.sent(&Sent {
            id: ctx.id,
            host,
            protocol,
            elapsed,
        });
    });
}

/// Report an arrived response head.
pub(crate) fn head(host: &str, status: u16, protocol: HttpVersion, elapsed: Duration) {
    with(|ctx| {
        ctx.hook.head(&Head {
            id: ctx.id,
            host,
            status,
            protocol,
            elapsed,
        });
    });
}

/// Report the end of the attempt.
pub(crate) fn done(outcome: Result<(), &Error>) {
    with(|ctx| {
        ctx.hook.done(&Done {
            id: ctx.id,
            elapsed: ctx.start.elapsed(),
            outcome,
        });
    });
}

/// A [`Trace`] that writes every event as a `tracing` debug event under the `leyline::trace` target.
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
        tracing::debug!(target: "leyline::trace", id = ev.id, host = ev.host, protocol = ?ev.protocol, elapsed_ms = ms(ev.elapsed), "sent");
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

/// Milliseconds elapsed, saturating into `u32`.
fn ms(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}

/// A [`Trace`] that fills the same numbers as [`crate::ResponseTiming`], readable after the request through [`Timing::snapshot`]. Share one instance across requests by cloning the `Arc` you hand to [`crate::SessionBuilder::trace`]; the snapshot then reports the most recent attempt.
#[derive(Debug, Default)]
pub struct Timing {
    reused: AtomicBool,
    fresh: AtomicBool,
    connect_ms: AtomicU32,
    send_ms: AtomicU32,
    total_ms: AtomicU32,
}

impl Timing {
    /// A recorder with every number at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// The numbers recorded by the most recent attempt.
    pub fn snapshot(&self) -> ResponseTiming {
        ResponseTiming {
            reused: self.reused.load(Ordering::Relaxed),
            connect_ms: self
                .fresh
                .load(Ordering::Relaxed)
                .then(|| self.connect_ms.load(Ordering::Relaxed)),
            send_ms: self.send_ms.load(Ordering::Relaxed),
            total_ms: self.total_ms.load(Ordering::Relaxed),
        }
    }
}

impl Trace for Timing {
    fn connect(&self, ev: &Connect<'_>) {
        self.reused.store(ev.reused, Ordering::Relaxed);
        self.fresh.store(!ev.reused, Ordering::Relaxed);
        self.connect_ms.store(ms(ev.elapsed), Ordering::Relaxed);
    }

    fn head(&self, ev: &Head<'_>) {
        self.send_ms.store(ms(ev.elapsed), Ordering::Relaxed);
    }

    fn done(&self, ev: &Done<'_>) {
        self.total_ms.store(ms(ev.elapsed), Ordering::Relaxed);
    }
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
