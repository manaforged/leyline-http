use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::sync::futures::Notified;

const SHARDS: usize = 32;
const COARSE_TICK: Duration = Duration::from_millis(250);

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
    ticker_live: AtomicBool,
    saw_activity: AtomicBool,
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
        self.saw_activity.store(true, Ordering::SeqCst);
        if !self.ticker_live.load(Ordering::SeqCst) {
            self.start_ticker();
        }
    }

    #[cold]
    fn start_ticker(self: &Arc<Self>) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        if self
            .ticker_live
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let weak: Weak<Self> = Arc::downgrade(self);
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            self.ticker_live.store(false, Ordering::SeqCst);
            return;
        };
        handle.spawn(async move {
            let _guard = TickerGuard(weak.clone());
            let mut idle_ticks = 0u8;
            loop {
                tokio::time::sleep(COARSE_TICK).await;
                let Some(shutdown) = weak.upgrade() else {
                    return;
                };
                if shutdown.is_stopped() {
                    return;
                }
                let active = shutdown.saw_activity.swap(false, Ordering::SeqCst);
                shutdown.wake_all();
                if active {
                    idle_ticks = 0;
                    continue;
                }
                idle_ticks += 1;
                if idle_ticks >= 2 {
                    shutdown.ticker_live.store(false, Ordering::SeqCst);
                    if shutdown.saw_activity.load(Ordering::SeqCst)
                        && shutdown
                            .ticker_live
                            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                            .is_ok()
                    {
                        idle_ticks = 0;
                        continue;
                    }
                    return;
                }
            }
        });
    }
}

struct TickerGuard(Weak<Shutdown>);

impl Drop for TickerGuard {
    fn drop(&mut self) {
        if let Some(shutdown) = self.0.upgrade() {
            shutdown.ticker_live.store(false, Ordering::SeqCst);
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
