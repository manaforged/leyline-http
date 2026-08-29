use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use bytes::Bytes;
use tokio::io::AsyncWrite;
use tokio::sync::mpsc;

use super::{H2ConnectStream, ShutdownState};

struct CountWaker(AtomicUsize);

impl Wake for CountWaker {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

// Lost-wakeup gate: `poll_write` must not build a fresh `reserve()`
// future on every poll. Doing so means returning `Pending` drops the
// future and deregisters the waker from the channel's waitlist — the
// task is never repolled when capacity frees and the write hangs
// forever under backpressure.
#[test]
fn poll_write_backpressure_wakes_when_capacity_frees() {
    let (write_tx, mut write_rx) = mpsc::channel::<io::Result<Bytes>>(1);
    // Fill the only slot so the next reserve must park.
    write_tx.try_send(Ok(Bytes::from_static(b"fill"))).unwrap();
    let (_read_tx, read_rx) = mpsc::channel::<io::Result<Bytes>>(1);

    let mut stream = H2ConnectStream {
        shutdown_state: ShutdownState::Open,
        status: 200,
        response_headers: Vec::new(),
        write_tx: Some(tokio_util::sync::PollSender::new(write_tx)),
        read_rx,
        read_leftover: Bytes::new(),
        read_eof: false,
    };

    let woken = Arc::new(CountWaker(AtomicUsize::new(0)));
    let waker = Waker::from(woken.clone());
    let mut cx = Context::from_waker(&waker);

    assert!(
        Pin::new(&mut stream)
            .poll_write(&mut cx, b"hello")
            .is_pending(),
        "first write must hit backpressure"
    );

    // Free the slot; the channel wakes registered reservers.
    assert!(write_rx.try_recv().is_ok());
    assert!(
        woken.0.load(Ordering::SeqCst) > 0,
        "freeing channel capacity must wake the parked writer"
    );

    // The retried write now completes.
    match Pin::new(&mut stream).poll_write(&mut cx, b"hello") {
        Poll::Ready(Ok(n)) => assert_eq!(n, 5),
        other => panic!("expected Ready(Ok(5)), got {other:?}"),
    }
}
