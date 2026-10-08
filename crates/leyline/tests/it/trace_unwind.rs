use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use leyline::Session;
use leyline::testing::{TestResponse, TestServer, queue};
use leyline::trace::{BodyEnd, Fanout, Trace};

struct PanickingSink;

impl Trace for PanickingSink {
    fn body(&self, _: &BodyEnd<'_>) {
        panic!("sink failed");
    }
}

#[derive(Default)]
struct Seen(AtomicBool);

impl Trace for Seen {
    fn body(&self, _: &BodyEnd<'_>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn a_streamed_body_dropped_while_unwinding_does_not_abort() {
    let server = TestServer::http(queue([TestResponse::new(200).close().body("unread")]))
        .await
        .unwrap();
    let seen = Arc::new(Seen::default());
    let session = Session::builder()
        .trace(Fanout::new().with(PanickingSink).with(Arc::clone(&seen)))
        .build()
        .unwrap();
    let resp = session.get(server.url("/")).stream().await.unwrap();
    let unwound = catch_unwind(AssertUnwindSafe(move || {
        let _held = resp;
        panic!("caller failed");
    }));
    assert!(unwound.is_err());
    assert!(!seen.0.load(Ordering::SeqCst));
}
