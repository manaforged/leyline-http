use std::time::Duration;

const WAIT_LIMIT: Duration = Duration::from_secs(5);

pub async fn until(mut done: impl FnMut() -> bool) {
    tokio::time::timeout(WAIT_LIMIT, async {
        while !done() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the condition did not hold in time");
}
