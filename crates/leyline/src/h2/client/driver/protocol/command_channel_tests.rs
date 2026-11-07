use std::mem::size_of;

use super::{CommandSlot, DriverCommand, command_channel};

#[tokio::test]
async fn command_channel_stores_a_box() {
    assert!(size_of::<DriverCommand>() > size_of::<CommandSlot>());
    assert_eq!(size_of::<CommandSlot>(), size_of::<usize>());
    let (tx, mut rx) = command_channel(1);
    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    tx.send(DriverCommand::Ping {
        ack_tx,
        deadline: std::time::Instant::now(),
    })
    .await
    .unwrap();
    let DriverCommand::Ping { ack_tx, .. } = rx.try_recv().unwrap() else {
        panic!("channel returned a different command");
    };
    ack_tx.send(()).unwrap();
    ack_rx.await.unwrap();
}
