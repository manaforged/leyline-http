use std::future::Future;

use super::SessionBuilder;
use crate::core::session::Session;
use crate::error::Result;

impl SessionBuilder {
    /// Runs `work` once per CPU core, each on its own single-threaded runtime with a session that shares cookies, cache, tokens and HSTS but keeps its own connections, and returns every core's output.
    pub fn run_per_core<F, Fut, T>(self, work: F) -> Result<Vec<T>>
    where
        F: Fn(Session) -> Fut + Sync,
        Fut: Future<Output = T>,
        T: Send,
    {
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        let root = self.build()?;
        std::thread::scope(|scope| {
            let threads: Vec<_> = (0..cores)
                .map(|_| {
                    let (session, work) = (root.per_core(), &work);
                    scope.spawn(move || -> Result<T> {
                        let runtime = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()?;
                        Ok(runtime.block_on(work(session)))
                    })
                })
                .collect();
            threads
                .into_iter()
                .map(|thread| {
                    thread
                        .join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                })
                .collect()
        })
    }
}
