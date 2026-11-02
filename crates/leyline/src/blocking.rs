//! Blocking use: every call runs on the calling thread.

use std::cell::Cell;
use std::ops::Deref;
use std::task::{Context, Poll, Waker};

use crate::{Result, SessionBuilder};

const SKIP_AFTER_PENDING: u8 = 16;

/// A session whose calls block the calling thread until they finish.
pub struct Session {
    session: crate::Session,
    runtime: tokio::runtime::Runtime,
    skips: Cell<u8>,
}

impl Session {
    /// Builds a blocking session from `builder`.
    pub fn new(builder: SessionBuilder) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let session = {
            let _entered = runtime.enter();
            builder.blocking_io().build()?
        };
        Ok(Self {
            session,
            runtime,
            skips: Cell::new(0),
        })
    }

    /// Runs `future` to completion on the calling thread and returns its output.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let skips = self.skips.get();
        if skips > 0 {
            self.skips.set(skips - 1);
            return self.runtime.block_on(future);
        }
        let first = {
            let _entered = self.runtime.enter();
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
        };
        match first {
            Poll::Ready(output) => output,
            Poll::Pending => {
                self.skips.set(SKIP_AFTER_PENDING);
                self.runtime.block_on(future)
            }
        }
    }
}

impl Deref for Session {
    type Target = crate::Session;

    fn deref(&self) -> &crate::Session {
        &self.session
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("blocking::Session")
            .field(&self.session)
            .finish()
    }
}

#[cfg(all(test, feature = "test-util"))]
mod tests;
