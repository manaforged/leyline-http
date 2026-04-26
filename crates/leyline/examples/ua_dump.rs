//! Dumps on-the-wire UA / sec-ch-ua / dnt / sec-gpc for each ChromiumBrand.
#![allow(missing_docs)]
use leyline::profile::Preset;
use leyline::Session;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn capture(session: Session) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut req = Vec::new();
        let mut tmp = [0u8; 2048];
        loop {
            let n = sock.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            req.extend_from_slice(&tmp[..n]);
            if req.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
            .await
            .unwrap();
        String::from_utf8_lossy(&req).to_string()
    });
    let _ = session
        .get(&format!("http://{addr}/"))
        .preset(Preset::Navigate)
        .send()
        .await
        .unwrap();
    server.await.unwrap()
}

fn extract(req: &str, header: &str) -> String {
    req.lines()
        .find(|l| {
            l.to_lowercase()
                .starts_with(&format!("{}:", header.to_lowercase()))
        })
        .map(|l| l.split_once(':').map_or("", |x| x.1).trim().to_string())
        .unwrap_or_else(|| "<missing>".into())
}

#[tokio::main]
async fn main() {
    for (name, sess) in [
        ("Chrome", Session::chrome_latest().unwrap()),
        ("Edge", Session::edge_latest().unwrap()),
        ("Brave", Session::brave_latest().unwrap()),
        ("Opera", Session::opera_latest().unwrap()),
        ("Vivaldi", Session::vivaldi_latest().unwrap()),
    ] {
        let req = capture(sess).await;
        println!("=== {name} ===");
        println!("  UA:        {}", extract(&req, "User-Agent"));
        println!("  sec-ch-ua: {}", extract(&req, "sec-ch-ua"));
        println!("  dnt:       {}", extract(&req, "dnt"));
        println!("  sec-gpc:   {}", extract(&req, "sec-gpc"));
        println!("  accept:    {}", extract(&req, "accept"));
        println!();
    }
}
