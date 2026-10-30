use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use leyline::Session;
use leyline::testing::{self, queue};
use leyline::trace::{Event, Trace};

struct PanickingSink;

impl Trace for PanickingSink {
    fn event(&self, event: &Event<'_>) {
        if let Event::Body { .. } = event {
            panic!("sink failed");
        }
    }
}

#[derive(Default)]
struct Seen(AtomicBool);

impl Trace for Seen {
    fn event(&self, event: &Event<'_>) {
        if let Event::Body { .. } = event {
            self.0.store(true, Ordering::SeqCst);
        }
    }
}

#[tokio::test]
async fn a_streamed_body_dropped_while_unwinding_does_not_abort() {
    let server = testing::Server::http(queue([testing::Response::new(200).close().body("unread")]))
        .await
        .unwrap();
    let seen = Arc::new(Seen::default());
    let session = Session::builder()
        .trace(PanickingSink)
        .trace(Arc::clone(&seen))
        .build()
        .unwrap();
    let resp = session.get(server.url("/")).stream().send().await.unwrap();
    let unwound = catch_unwind(AssertUnwindSafe(move || {
        let _held = resp;
        panic!("caller failed");
    }));
    assert!(unwound.is_err());
    assert!(!seen.0.load(Ordering::SeqCst));
}
