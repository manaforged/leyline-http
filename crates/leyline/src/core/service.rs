use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::core::{Request, Response, Result, Session};

/// Tower-compatible adapter for dispatching owned Leyline requests.
#[derive(Clone)]
pub struct LeylineService {
    session: Session,
}

impl LeylineService {
    /// Wrap a session as a Tower service.
    pub fn new(session: Session) -> Self {
        Self { session }
    }

    /// Borrow the underlying session.
    pub fn session(&self) -> &Session {
        &self.session
    }
}

impl tower_service::Service<Request> for LeylineService {
    type Response = Response;
    type Error = crate::core::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let session = self.session.clone();
        Box::pin(async move { session.execute(req).await })
    }
}
