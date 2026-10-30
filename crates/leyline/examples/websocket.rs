use leyline::Session;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "wss://echo.websocket.org".to_string());

    let session = Session::from(leyline::Browser::default());
    let mut ws = session.get(&url).websocket().await?;
    ws.send(leyline::Message::Text("hello from leyline".to_owned()))
        .await?;
    if let Some(msg) = ws.recv().await? {
        println!("{msg:?}");
    }
    ws.close().await
}
