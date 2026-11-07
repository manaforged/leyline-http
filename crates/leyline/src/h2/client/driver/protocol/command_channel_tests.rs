use std::sync::Arc;

use super::{DriverCommand, Head, ResponseSink, command_channel};
use crate::h2::H2Error;
use crate::h2::connection::PseudoHeaders;
use crate::wire::Body;

#[tokio::test]
async fn command_channel_stores_a_box() {
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

fn ping() -> DriverCommand {
    let (ack, _) = tokio::sync::oneshot::channel();
    DriverCommand::Ping((ack, std::time::Instant::now()))
}

async fn let_senders_park() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn a_batch_drain_wakes_every_waiting_sender() {
    let (tx, mut rx) = command_channel(2);
    tx.send(ping()).await.unwrap();
    tx.send(ping()).await.unwrap();
    let waiting: Vec<_> = (0..8)
        .map(|_| {
            let tx = tx.clone();
            tokio::spawn(async move { tx.send(ping()).await.is_ok() })
        })
        .collect();
    let_senders_park().await;
    rx.try_recv().unwrap();
    rx.try_recv().unwrap();
    for _ in 0..8 {
        let next = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            std::future::poll_fn(|cx| rx.poll_recv(cx)),
        );
        assert!(
            next.await.is_ok(),
            "a sender stayed parked with room in the queue"
        );
    }
    for sender in waiting {
        assert!(sender.await.unwrap());
    }
}

#[tokio::test]
async fn closing_the_driver_side_fails_every_waiting_sender() {
    let (tx, rx) = command_channel(1);
    tx.send(ping()).await.unwrap();
    let waiting: Vec<_> = (0..4)
        .map(|_| {
            let tx = tx.clone();
            tokio::spawn(async move { tx.send(ping()).await.is_err() })
        })
        .collect();
    let_senders_park().await;
    drop(rx);
    for sender in waiting {
        let failed = tokio::time::timeout(std::time::Duration::from_secs(1), sender);
        assert!(
            failed
                .await
                .expect("a sender stayed parked after the driver ended")
                .unwrap()
        );
    }
}
