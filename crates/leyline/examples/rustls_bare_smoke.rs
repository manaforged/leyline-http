//! Functional smoke for the bare rustls backend. Run with:
//! `cargo run -p leyline --example rustls_bare_smoke --features tls-rustls`
//! Hits public test hosts to confirm the rustls handshake + fetch.

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session = leyline::Session::new();

    // h2 GET (default ALPN) — example.com negotiates HTTP/2.
    let resp = session.get("https://example.com/").send().await?;
    println!("example.com  status={}", resp.status());
    assert_eq!(resp.status(), 200, "example.com should return 200");
    let body = resp.into_text();
    assert!(body.contains("Example Domain"), "unexpected body");

    // Second host: fresh handshake to a different origin.
    let resp2 = session
        .get("https://www.cloudflare.com/cdn-cgi/trace")
        .send()
        .await?;
    println!("cloudflare trace  status={}", resp2.status());
    assert_eq!(resp2.status(), 200);

    println!("OK: bare rustls backend handshakes + fetches over h2");
    Ok(())
}
