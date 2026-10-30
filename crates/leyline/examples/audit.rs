use leyline::{Browser, Fingerprint, Platform, Session};

const ECHO_URL: &str = "https://tls.peet.ws/api/all";

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::default())
        .platform(Platform::Linux)
        .audit(true)
        .build()?;

    let resp = session.get(ECHO_URL).send().await?;
    let Some(sent) = resp.fingerprint().cloned() else {
        eprintln!("no fingerprint on this response");
        return Ok(());
    };
    println!("JA4:  {}", sent.ja4());
    println!("JA4T: {}", sent.ja4t());
    println!("JA4H: {}", sent.ja4h());

    let observed = Fingerprint::from_json(&resp.text().await?)?;
    let diff = sent.diff(&observed);
    for (field, expected, seen) in &diff {
        println!("{field:<15} sent {expected}, echo saw {seen}");
    }
    println!("fingerprint matches: {}", diff.is_empty());
    Ok(())
}
