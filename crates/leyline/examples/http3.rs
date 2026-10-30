use leyline::{Browser, Session};

const URL: &str = "https://www.cloudflare.com/";
const SECOND_URL: &str = "https://www.cloudflare.com/cdn-cgi/trace";

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::from(Browser::default());

    let first = session.get(URL).send().await?;
    println!(
        "first: {} over {:?}, alt-svc {:?}",
        first.status(),
        first.version(),
        first.header("alt-svc")
    );

    let second = session.get(SECOND_URL).send().await?;
    println!("second: {} over {:?}", second.status(), second.version());
    Ok(())
}
