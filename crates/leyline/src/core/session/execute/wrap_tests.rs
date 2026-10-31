use std::mem::size_of_val;

use crate::Session;
use crate::core::deadline::within;

async fn heavy() -> crate::Result<()> {
    let buf = [7u8; 4096];
    tokio::task::yield_now().await;
    std::hint::black_box(&buf);
    Ok(())
}

#[tokio::test]
async fn wrappers_hold_their_future_once() {
    let session = Session::new();
    let deadline = session.deadline(None);
    let limit = size_of_val(&heavy()) + 256;
    assert!(size_of_val(&within(None, heavy())) < limit);
    assert!(size_of_val(&deadline.total(heavy())) < limit);
    assert!(size_of_val(&deadline.response_header(std::pin::pin!(heavy()))) < limit);
    assert!(size_of_val(&session.unless_shut_down(heavy())) < limit);
    assert!(size_of_val(&crate::trace::scope(None, heavy())) < limit);
}
