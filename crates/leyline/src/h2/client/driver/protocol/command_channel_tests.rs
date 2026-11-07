use std::mem::size_of;
use std::sync::Arc;

use super::{CommandSlot, DriverCommand, Head, ResponseSink, command_channel};
use crate::h2::H2Error;
use crate::h2::connection::PseudoHeaders;
use crate::wire::Body;

#[tokio::test]
async fn command_channel_stores_a_box() {
    assert!(size_of::<DriverCommand>() > size_of::<CommandSlot>());
    assert_eq!(size_of::<CommandSlot>(), size_of::<usize>());
    let (tx, mut rx) = command_channel(1);
    let (sink_tx, sink_rx) = tokio::sync::oneshot::channel();
    tx.send(DriverCommand::SendRequest {
        head: Arc::new(Head::new(PseudoHeaders::default(), Vec::new())),
        body: Body::default(),
        sink: ResponseSink::Buffered(sink_tx),
        slot: None,
    })
    .await
    .unwrap();
    let ResponseSink::Buffered(sink_tx) = rx.try_recv().unwrap().into_sink().unwrap() else {
        panic!("channel returned a different command");
    };
    if sink_tx.send(Err(H2Error::NotSent)).is_err() {
        panic!("the response receiver closed");
    }
    assert!(matches!(sink_rx.await, Ok(Err(H2Error::NotSent))));
}
