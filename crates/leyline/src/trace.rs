use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures_util::future::Either;

use crate::{Error, HttpVersion};

mod body;
mod event;
mod fanout;
mod headers;
mod metrics;
mod notes;
mod summary;

pub(crate) use body::BodyWatch;
pub use event::Event;
pub(crate) use fanout::Fanout;
pub(crate) use headers::masked;
pub use metrics::Metrics;
pub(crate) use notes::{note_browser, note_label, note_proxy};
pub(crate) use summary::{Finish, summary};

#[doc = include_str!("../docs/trace.md")]
pub trait Trace: Send + Sync + 'static {
    /// Receives one step of a traced request.
    fn event(&self, event: &Event<'_>);
}

#[derive(Clone)]
pub(crate) struct Ctx {
    hook: Arc<dyn Trace>,
    id: u64,
    start: Instant,
    root: (u64, Instant),
    notes: notes::SharedNotes,
}

tokio::task_local! {
    static CURRENT: Ctx;
}

static NEXT: AtomicU64 = AtomicU64::new(1);

pub(crate) fn scope<F>(
    hook: Option<&Arc<dyn Trace>>,
    fut: F,
) -> impl Future<Output = F::Output> + use<F>
where
    F: Future,
{
    match hook {
        Some(hook) => {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let start = Instant::now();
            let root = CURRENT.try_with(|outer| outer.root).unwrap_or((id, start));
            let ctx = Ctx {
                hook: Arc::clone(hook),
                id,
                start,
                root,
                notes: notes::SharedNotes::default(),
            };
            Either::Left(CURRENT.scope(ctx, fut))
        }
        None => Either::Right(fut),
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
        ctx.hook.event(&Event::Dns {
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
        ctx.hook.event(&Event::Connect {
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
        ctx.hook.event(&Event::Tls {
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
        ctx.hook.event(&Event::Sent {
            id: ctx.id,
            host,
            method,
            path,
            protocol: protocol.into(),
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
        ctx.hook.event(&Event::Head {
            id: ctx.id,
            host,
            status,
            protocol: protocol.into(),
            elapsed,
            headers: &headers,
        });
    });
}

pub(crate) fn done(outcome: Result<(), &Error>) {
    with(|ctx| {
        ctx.hook.event(&Event::Done {
            id: ctx.id,
            elapsed: ctx.start.elapsed(),
            outcome,
        });
    });
}

/// Writes every event to the `tracing` crate under the `leyline::trace` target.
#[derive(Debug, Default, Clone, Copy)]
pub struct Log;

impl Trace for Log {
    fn event(&self, event: &Event<'_>) {
        match *event {
            Event::Dns {
                id,
                host,
                port,
                addrs,
                elapsed,
            } => {
                tracing::debug!(target: "leyline::trace", id, host, port, addrs, elapsed_ms = ms(elapsed), "dns");
            }
            Event::Connect {
                id,
                host,
                port,
                reused,
                elapsed,
            } => {
                tracing::debug!(target: "leyline::trace", id, host, port, reused, elapsed_ms = ms(elapsed), "connect");
            }
            Event::Tls {
                id,
                host,
                version,
                cipher,
                alpn,
                elapsed,
            } => {
                tracing::debug!(target: "leyline::trace", id, host, version, cipher, alpn, elapsed_ms = ms(elapsed), "tls");
            }
            Event::Sent {
                id,
                host,
                method,
                path,
                protocol,
                elapsed,
            } => {
                tracing::debug!(target: "leyline::trace", id, host, method, path = %crate::util::mask_query(path), protocol = ?protocol, elapsed_ms = ms(elapsed), "sent");
            }
            Event::Head {
                id,
                host,
                status,
                protocol,
                elapsed,
                ..
            } => {
                tracing::debug!(target: "leyline::trace", id, host, status, protocol = ?protocol, elapsed_ms = ms(elapsed), "head");
            }
            Event::Done {
                id,
                elapsed,
                outcome: Ok(()),
            } => {
                tracing::debug!(target: "leyline::trace", id, elapsed_ms = ms(elapsed), "done");
            }
            Event::Done {
                id,
                elapsed,
                outcome: Err(e),
            } => {
                tracing::debug!(target: "leyline::trace", id, elapsed_ms = ms(elapsed), kind = ?e.kind(), error = %e, "done");
            }
            Event::Summary { .. } => summary::render(event),
            Event::Body {
                id,
                bytes,
                elapsed,
                outcome,
            } => body::render(id, bytes, elapsed, outcome),
        }
    }
}

fn ms(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}

impl<T: Trace> Trace for Arc<T> {
    fn event(&self, event: &Event<'_>) {
        T::event(self, event);
    }
}

#[cfg(test)]
mod tests;
