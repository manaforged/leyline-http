use leyline::Session;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "wss://echo.websocket.org".to_string());

    let session = Session::new();
    let mut ws = session.websocket(&url).connect().await?;
    ws.send(leyline::WsMessage::Text("hello from leyline".to_owned()))
        .await?;
    if let Some(msg) = ws.recv().await? {
        println!("{msg:?}");
    }
    ws.close().await
}
