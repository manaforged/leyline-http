//! `tower::Service` adapter for Leyline.
//!
//! Lets a [`leyline::Session`] plug into tower / axum middleware
//! stacks: rate-limiters, circuit breakers, load-balancers, tracing
//! wrappers. The adapter takes a [`leyline::Request`] value and
//! dispatches it through the session.
//!
//! ```rust,ignore
//! use leyline::{Request, Session};
//! use leyline_tower::LeylineService;
//! use tower::{Service, ServiceExt};
//!
//! let session = Session::chrome_latest()?;
//! let mut svc = LeylineService::new(session);
//! let resp = svc.ready().await?.call(Request::get("https://example.com")).await?;
//! ```
//!
//! # Cloning
//!
//! `LeylineService` is cheaply `Clone`: the underlying session lives
//! behind an `Arc`, so cloning the service yields a handle to the
//! same connection pool and cookie jar. That's usually what you want
//! for load-balancing stacks — each worker gets its own
//! `Service` value but shares the session.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use leyline::{Error, Request, Response, Session};
use tower::Service;

/// `tower::Service` over a [`Session`].
///
/// Build one with [`LeylineService::new`]. The underlying session is
/// held behind an `Arc`, so `Clone` is cheap and lets tower stacks
/// hand copies to worker tasks.
pub struct LeylineService {
    session: Arc<Session>,
}

impl LeylineService {
    /// Wrap a session as a tower service.
    pub fn new(session: Session) -> Self {
        Self {
            session: Arc::new(session),
        }
    }

    /// Wrap an already-shared session as a tower service. Useful when
    /// the caller already holds the session in an `Arc` (e.g. for
    /// cross-task sharing) and doesn't want the double-wrapping.
    pub fn from_arc(session: Arc<Session>) -> Self {
        Self { session }
    }

    /// Clone-on-demand access to the underlying session.
    pub fn session(&self) -> &Session {
        &self.session
    }
}

impl Clone for LeylineService {
    fn clone(&self) -> Self {
        Self {
            session: Arc::clone(&self.session),
        }
    }
}

impl Service<Request> for LeylineService {
    type Response = Response;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let session = Arc::clone(&self.session);
        Box::pin(async move { session.execute_request(req).await })
    }
}
