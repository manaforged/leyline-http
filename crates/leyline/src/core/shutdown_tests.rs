use std::collections::HashSet;
use std::sync::Arc;

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_wakes_waiters_on_every_thread() {
    let shutdown = Arc::new(Shutdown::default());
    let (ready_tx, mut ready_rx) = tokio::sync::mpsc::channel(64);
    let waiters: Vec<_> = (0..64)
        .map(|_| {
            let shutdown = Arc::clone(&shutdown);
            let ready_tx = ready_tx.clone();
            tokio::spawn(async move {
                let mut wait = std::pin::pin!(shutdown.wait());
                assert!(futures_util::poll!(wait.as_mut()).is_pending());
                ready_tx.send(()).await.unwrap();
                wait.await;
            })
        })
        .collect();
    for _ in 0..64 {
        ready_rx.recv().await.unwrap();
    }
    shutdown.stop();
    for waiter in waiters {
        waiter.await.unwrap();
    }
}

#[test]
fn threads_wait_on_different_shards() {
    let shards: HashSet<usize> = (0..4)
        .map(|_| std::thread::spawn(shard).join().unwrap())
        .collect();
    assert_eq!(shards.len(), 4);
}

#[tokio::test]
async fn a_wait_after_stop_is_ready() {
    let shutdown = Shutdown::default();
    shutdown.stop();
    assert!(shutdown.is_stopped());
    shutdown.wait().await;
}
