use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::sync::futures::Notified;

const SHARDS: usize = 32;
const COARSE_TICK: Duration = Duration::from_millis(250);
const TICKER_IDLE: u8 = 0;
const TICKER_QUIET: u8 = 1;
const TICKER_ACTIVE: u8 = 2;

pub(crate) type Stopped = Pin<Box<dyn Future<Output = ()> + Send + Sync>>;

#[repr(align(64))]
#[derive(Default)]
struct PaddedNotify(Notify);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wake {
    Stopped,
    Tick,
}

#[derive(Default)]
pub(crate) struct Shutdown {
    stopped: AtomicBool,
    ticker: AtomicU8,
    wake: [PaddedNotify; SHARDS],
}

impl Shutdown {
    pub(crate) fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.wake_all();
    }

    fn wake_all(&self) {
        for slot in &self.wake {
            slot.0.notify_waiters();
        }
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    pub(crate) fn wait(&self) -> Wait<'_> {
        let idx = shard();
        Wait {
            shutdown: self,
            shard: idx,
            notified: self.wake[idx].0.notified(),
        }
    }

    pub(crate) fn wait_owned(self: &Arc<Self>) -> Stopped {
        let shutdown = Arc::clone(self);
        Box::pin(async move { shutdown.wait().await })
    }

    pub(crate) fn note_coarse(self: &Arc<Self>) {
        match self.ticker.load(Ordering::Relaxed) {
            TICKER_ACTIVE => {}
            TICKER_QUIET => self.mark_active_or_start(),
            _ => self.start_ticker(),
        }
    }

    #[cold]
    fn mark_active_or_start(self: &Arc<Self>) {
        if let Err(TICKER_IDLE) = self.ticker.compare_exchange(
            TICKER_QUIET,
            TICKER_ACTIVE,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            self.start_ticker();
        }
    }

    #[cold]
    fn start_ticker(self: &Arc<Self>) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        if self
            .ticker
            .compare_exchange(
                TICKER_IDLE,
                TICKER_ACTIVE,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return;
        }
        let weak: Weak<Self> = Arc::downgrade(self);
        let mut guard = TickerGuard(Some(weak.clone()));
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        handle.spawn(async move {
            let mut idle_ticks = 0u8;
            loop {
                tokio::time::sleep(COARSE_TICK).await;
                let Some(shutdown) = weak.upgrade() else {
                    return;
                };
                if shutdown.is_stopped() {
                    return;
                }
                let prev = shutdown.ticker.swap(TICKER_QUIET, Ordering::AcqRel);
                shutdown.wake_all();
                if prev == TICKER_ACTIVE {
                    idle_ticks = 0;
                    continue;
                }
                idle_ticks += 1;
                if idle_ticks >= 2 {
                    match shutdown.ticker.compare_exchange(
                        TICKER_QUIET,
                        TICKER_IDLE,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => {
                            guard.disarm();
                            return;
                        }
                        Err(TICKER_ACTIVE) => {
                            idle_ticks = 0;
                            continue;
                        }
                        Err(_) => {
                            guard.disarm();
                            return;
                        }
                    }
                }
            }
        });
    }
}

struct TickerGuard(Option<Weak<Shutdown>>);

impl TickerGuard {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for TickerGuard {
    fn drop(&mut self) {
        if let Some(shutdown) = self.0.as_ref().and_then(Weak::upgrade) {
            shutdown.ticker.store(TICKER_IDLE, Ordering::Release);
        }
    }
}

fn shard() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        static SHARD: usize = NEXT.fetch_add(1, Ordering::Relaxed) & (SHARDS - 1);
    }
    SHARD.with(|shard| *shard)
}

pin_project_lite::pin_project! {
    pub(crate) struct Wait<'a> {
        shutdown: &'a Shutdown,
        shard: usize,
        #[pin]
        notified: Notified<'a>,
    }
}

impl<'a> Wait<'a> {
    pub(crate) fn shutdown(&self) -> &'a Shutdown {
        self.shutdown
    }

    pub(crate) fn poll_wake(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Wake> {
        let mut this = self.project();
        if this.shutdown.is_stopped() {
            return Poll::Ready(Wake::Stopped);
        }
        match this.notified.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(()) => {
                if this.shutdown.is_stopped() {
                    Poll::Ready(Wake::Stopped)
                } else {
                    let shard = *this.shard;
                    let shutdown = *this.shutdown;
                    this.notified.set(shutdown.wake[shard].0.notified());
                    Poll::Ready(Wake::Tick)
                }
            }
        }
    }
}

impl Future for Wait<'_> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        loop {
            match self.as_mut().poll_wake(cx) {
                Poll::Ready(Wake::Stopped) => return Poll::Ready(()),
                Poll::Ready(Wake::Tick) => continue,
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
#[path = "shutdown_tests.rs"]
mod tests;
