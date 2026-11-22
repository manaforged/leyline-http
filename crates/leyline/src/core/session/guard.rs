use super::Session;
use crate::core::BodyRules;
use crate::core::request::Draft;
use crate::core::response::Response;
use crate::core::shutdown::{Shutdown, Wake};
use crate::deadline::{Deadline, Timeout, Timer};
use crate::error::{Error, Kind, Result};
use crate::trace;
use crate::util::redact;
use crate::wire::ErrorBudget;

impl Session {
    pub(crate) fn deadline(&self, request: Option<&Timeout>) -> Deadline {
        Deadline::new(&self.inner.inputs.timeouts, request)
    }

    pub(crate) fn error_budget(deadline: &Deadline, rules: &BodyRules) -> ErrorBudget {
        ErrorBudget::new(
            deadline.error_body_wait(),
            rules.max_error_body.min(rules.max_body_size),
        )
    }

    pub(crate) fn attempt(
        &self,
        attempt: Box<Draft>,
    ) -> impl std::future::Future<Output = Result<Response>> + '_ {
        let timer = attempt.deadline().timer();
        let span = tracing::debug_span!(
            "session.execute",
            http.method = attempt.method().as_str(),
            http.url = redact(attempt.url().map_or("", url::Url::as_str)),
            pool.hit = tracing::field::Empty,
        );
        let fut: std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Response>> + Send + '_>,
        > = Box::pin(self.execute_inner(attempt));
        trace::scope(
            self.inner.inputs.trace.as_ref(),
            Guarded {
                shutdown: &self.inner.shutdown,
                stop: self.inner.shutdown.wait(),
                fut,
                timer,
                span,
            },
        )
    }

    pub(crate) fn unless_shut_down<T, F>(&self, fut: F) -> UnlessShutDown<'_, F>
    where
        F: std::future::Future<Output = Result<T>>,
    {
        UnlessShutDown {
            stop: self.inner.shutdown.wait(),
            fut,
        }
    }

    /// Shuts the session down so that pending and new requests fail; await it to wait for the last autosave.
    pub fn shutdown(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        self.inner.shutdown.stop();
        let writers = std::mem::take(&mut *self.writers());
        async move {
            for writer in writers {
                drop(writer.await);
            }
        }
    }

    pub(crate) fn track_writer(&self, writer: tokio::task::JoinHandle<()>) {
        let mut writers = self.writers();
        writers.retain(|running| !running.is_finished());
        writers.push(writer);
    }

    fn writers(&self) -> std::sync::MutexGuard<'_, Vec<tokio::task::JoinHandle<()>>> {
        crate::util::lock(&self.inner.writers)
    }

    pub(crate) fn stopped(&self) -> crate::core::shutdown::Stopped {
        self.inner.shutdown.wait_owned()
    }

    /// Returns whether `shutdown` has been called on this session.
    pub fn is_shut_down(&self) -> bool {
        self.inner.shutdown.is_stopped()
    }

    pub(crate) fn traces(&self) -> bool {
        self.inner.inputs.trace.is_some()
    }

    pub(crate) fn traced<'a, F>(
        &'a self,
        method: &'a str,
        original_url: &'a str,
        streamed: bool,
        request: F,
    ) -> impl std::future::Future<Output = Result<Response>> + 'a
    where
        F: std::future::Future<Output = (Result<Response>, u32)> + 'a,
    {
        use futures_util::FutureExt;
        let hook = self.inner.inputs.trace.as_ref();
        trace::scope(
            hook,
            request.map(move |(out, attempts)| {
                if hook.is_some() {
                    let finish = trace::Finish {
                        method,
                        original_url,
                        attempts,
                        streamed,
                    };
                    let answer = out.as_ref().map(|response| trace::Answer {
                        url: response.url(),
                        redirects: response.redirect_chain().len(),
                        status: response.status(),
                        version: response.version(),
                    });
                    trace::summary(&finish, answer);
                }
                out
            }),
        )
    }
}

pin_project_lite::pin_project! {
    pub(crate) struct UnlessShutDown<'a, F> {
        #[pin]
        stop: crate::core::shutdown::Wait<'a>,
        #[pin]
        fut: F,
    }
}

impl<T, F> std::future::Future for UnlessShutDown<'_, F>
where
    F: std::future::Future<Output = Result<T>>,
{
    type Output = Result<T>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<T>> {
        let this = self.project();
        if this.stop.poll(cx).is_ready() {
            return std::task::Poll::Ready(Err(Error::shut_down()));
        }
        this.fut.poll(cx)
    }
}

pin_project_lite::pin_project! {
    struct Guarded<'a, F> {
        shutdown: &'a std::sync::Arc<Shutdown>,
        #[pin]
        stop: crate::core::shutdown::Wait<'a>,
        #[pin]
        fut: F,
        timer: Option<Timer>,
        span: tracing::Span,
    }
}

impl<F> std::future::Future for Guarded<'_, F>
where
    F: std::future::Future<Output = Result<Response>>,
{
    type Output = Result<Response>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<Response>> {
        let mut this = self.project();
        let at = this.timer.as_ref().map(Timer::deadline);
        let out = if this.stop.shutdown().is_stopped() {
            Err(Error::shut_down())
        } else if let std::task::Poll::Ready(out) =
            crate::deadline::expiring(at, || this.span.in_scope(|| this.fut.poll(cx)))
        {
            out
        } else {
            loop {
                match this.timer.as_mut() {
                    Some(Timer::Exact(sleep)) => {
                        if sleep.as_mut().poll(cx).is_ready() {
                            break Err(Error::new(Kind::Timeout));
                        }
                    }
                    Some(Timer::Coarse(at)) => {
                        if tokio::time::Instant::now() >= *at {
                            break Err(Error::new(Kind::Timeout));
                        }
                        this.shutdown.note_coarse();
                    }
                    None => {}
                }
                match this.stop.as_mut().poll_wake(cx) {
                    std::task::Poll::Ready(Wake::Stopped) => break Err(Error::shut_down()),
                    std::task::Poll::Ready(Wake::Tick) => continue,
                    std::task::Poll::Pending => return std::task::Poll::Pending,
                }
            }
        };
        trace::done(out.as_ref().map(|_| ()));
        std::task::Poll::Ready(out)
    }
}
