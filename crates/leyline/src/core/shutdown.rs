use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll};

use tokio::sync::Notify;
use tokio::sync::futures::Notified;

const SHARDS: usize = 32;

pub(crate) type Stopped = Pin<Box<dyn Future<Output = ()> + Send + Sync>>;

#[derive(Default)]
pub(crate) struct Shutdown {
    stopped: AtomicBool,
    wake: [Notify; SHARDS],
}

impl Shutdown {
    pub(crate) fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        for wake in &self.wake {
            wake.notify_waiters();
        }
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    pub(crate) fn wait(&self) -> Wait<'_> {
        Wait {
            shutdown: self,
            notified: self.wake[shard()].notified(),
        }
    }

    pub(crate) fn wait_owned(self: &Arc<Self>) -> Stopped {
        let shutdown = Arc::clone(self);
        Box::pin(async move { shutdown.wait().await })
    }
}

fn shard() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        static SHARD: usize = NEXT.fetch_add(1, Ordering::Relaxed) % SHARDS;
    }
    SHARD.with(|shard| *shard)
}

pin_project_lite::pin_project! {
    pub(crate) struct Wait<'a> {
        shutdown: &'a Shutdown,
        #[pin]
        notified: Notified<'a>,
    }
}

impl<'a> Wait<'a> {
    pub(crate) fn shutdown(&self) -> &'a Shutdown {
        self.shutdown
    }
}

impl Future for Wait<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.project();
        if this.shutdown.is_stopped() {
            return Poll::Ready(());
        }
        this.notified.poll(cx)
    }
}

#[cfg(test)]
#[path = "shutdown_tests.rs"]
mod tests;
