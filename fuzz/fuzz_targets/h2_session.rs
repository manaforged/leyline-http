#![no_main]

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use leyline::internals::h2::connection::PseudoHeaders;
use leyline::internals::h2::{H2Client, H2Config, Head, start};
use leyline::internals::pool::TransportBody;
use leyline::Body;
use libfuzzer_sys::fuzz_target;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, duplex, sink, split};
use tokio::runtime::Builder;
use tokio::time::timeout;

const HANG: Duration = Duration::from_secs(3600);
const EMPTY_SETTINGS: [u8; 9] = [0, 0, 0, 4, 0, 0, 0, 0, 0];
const PREFACE: usize = 24;
const HEADERS: u8 = 1;
const REQUESTS: [bool; 3] = [false, true, true];

fn head() -> Arc<Head> {
    let pseudo = PseudoHeaders {
        method: "GET".into(),
        scheme: "https".into(),
        authority: "example.test".into(),
        path: "/".into(),
        protocol: None,
    };
    Arc::new(Head::new(pseudo, Vec::new()))
}

async fn exchange(client: H2Client, streamed: bool) {
    let Ok(response) = client.send_shared(head(), Body::default(), streamed).await else {
        return;
    };
    if let TransportBody::Streaming(mut body) = response.body {
        while body.next().await.is_some() {}
    }
}

async fn requests_arrive(from_client: &mut (impl AsyncRead + Unpin)) -> std::io::Result<()> {
    let mut preface = [0; PREFACE];
    from_client.read_exact(&mut preface).await?;
    let mut seen = 0;
    while seen < REQUESTS.len() {
        let mut header = [0; 9];
        from_client.read_exact(&mut header).await?;
        let length = u32::from_be_bytes([0, header[0], header[1], header[2]]);
        tokio::io::copy(&mut from_client.take(u64::from(length)), &mut sink()).await?;
        seen += usize::from(header[3] == HEADERS);
    }
    Ok(())
}

async fn session(data: Vec<u8>) {
    let (near, far) = duplex(1 << 16);
    let (mut from_client, mut to_client) = split(far);
    tokio::spawn(async move {
        drop(to_client.write_all(&EMPTY_SETTINGS).await);
        drop(requests_arrive(&mut from_client).await);
        drop(to_client.write_all(&data).await);
        drop(to_client.shutdown().await);
        tokio::io::copy(&mut from_client, &mut sink()).await
    });
    let Ok(client) = start(near, H2Config::default()).await else {
        return;
    };
    let tasks = REQUESTS.map(|streamed| tokio::spawn(exchange(client.clone(), streamed)));
    for task in tasks {
        timeout(HANG, task).await.expect("a request hung after the peer closed").expect("request task");
    }
    timeout(HANG, exchange(client, false)).await.expect("a request hung on a closed connection");
}

fuzz_target!(|data: &[u8]| {
    let runtime = Builder::new_current_thread().enable_time().start_paused(true).build().expect("runtime");
    runtime.block_on(session(data.to_vec()));
});
