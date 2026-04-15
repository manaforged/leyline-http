//! Manual WebSocket smoke check using Leyline's profiled TLS path.

use leyline::Session;

#[tokio::main]
async fn main() {
    let session = Session::chrome().unwrap();

    println!("=== WebSocket Smoke Test ===\n");

    println!("Connecting to wss://echo.websocket.org ...");
    match session.websocket("wss://echo.websocket.org").await {
        Ok(mut ws) => {
            println!("  Connected!");
            ws.send("hello from leyline").await.unwrap();
            // First message is usually a server greeting, then echo
            if let Some(msg) = ws.recv().await.unwrap() {
                println!("  Recv: {msg}");
            }
            ws.send("echo this").await.unwrap();
            if let Some(msg) = ws.recv().await.unwrap() {
                println!("  Recv: {msg}");
            }
            ws.close().await.ok();
            println!("  ✓ WebSocket works");
        }
        Err(e) => println!("  ✗ Failed: {e}"),
    }
}
