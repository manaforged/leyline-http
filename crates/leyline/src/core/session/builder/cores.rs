use std::future::Future;

use super::SessionBuilder;
use crate::core::error::Result;
use crate::core::session::Session;

impl SessionBuilder {
    /// Runs `work` once per CPU core, each on its own single-threaded runtime with its own session from this builder, and returns every core's output.
    pub fn run_per_core<F, Fut, T>(self, work: F) -> Result<Vec<T>>
    where
        F: Fn(Session) -> Fut + Sync,
        Fut: Future<Output = T>,
        T: Send,
    {
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        std::thread::scope(|scope| {
            let threads: Vec<_> = (0..cores)
                .map(|_| {
                    let (builder, work) = (self.clone(), &work);
                    scope.spawn(move || -> Result<T> {
                        let runtime = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()?;
                        runtime.block_on(async move { Ok(work(builder.build()?).await) })
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
