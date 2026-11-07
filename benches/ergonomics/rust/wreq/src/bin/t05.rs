use futures::{StreamExt, TryStreamExt, stream};

#[tokio::main]
async fn main() -> wreq::Result<()> {
    let client = wreq::Client::new();
    let statuses: Vec<_> = stream::iter(0..100)
        .map(|i| {
            client
                .get(format!("http://127.0.0.1:18743/t05/item/{i}"))
                .send()
        })
        .buffer_unordered(10)
        .map_ok(|response| response.status().as_u16())
        .try_collect()
        .await?;
    let ok = statuses.iter().filter(|status| **status == 200).count();
    let missing = statuses.iter().filter(|status| **status == 404).count();
    println!("{ok} {missing}");
    Ok(())
}
