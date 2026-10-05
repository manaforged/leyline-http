#[path = "h2_support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use leyline::BrowserProfile;
use leyline::h2::frame::FrameType;
use leyline::h2::{H2Client, H2Config, RequestBody, ResponseBody, ResponseMode};
use support::{
    get_head, read_frame, read_preface, write_data, write_raw_headers, write_server_settings,
    write_settings_ack,
};
use tokio::io::DuplexStream;
use tokio::time::timeout;

async fn request_stream(peer: &mut DuplexStream) -> u32 {
    read_preface(peer).await;
    let (header, _) = read_frame(peer).await;
    assert_eq!(header.frame_type, FrameType::Settings as u8);
    write_server_settings(peer).await;
    write_settings_ack(peer).await;
    loop {
        let (header, _) = read_frame(peer).await;
        if header.frame_type == FrameType::Headers as u8 {
            return header.stream_id;
        }
    }
}

async fn client(peer_io: DuplexStream) -> H2Client {
    let config = H2Config::from_profile(&BrowserProfile::bare().h2).expect("bare H2 config");
    leyline::h2::start(peer_io, config)
        .await
        .expect("handshake")
}

#[tokio::test]
async fn an_error_prefix_success_is_buffered_with_its_trailers() {
    let (client_io, mut peer) = tokio::io::duplex(1 << 16);
    let server = tokio::spawn(async move {
        let stream_id = request_stream(&mut peer).await;
        write_raw_headers(&mut peer, stream_id, &[(":status", "200")], false).await;
        write_data(&mut peer, stream_id, b"ok", false).await;
        write_raw_headers(&mut peer, stream_id, &[("grpc-status", "0")], true).await;
        peer
    });
    let handle = client(client_io).await;

    let response = timeout(
        Duration::from_secs(5),
        handle.send_shared(
            Arc::new(get_head("/")),
            RequestBody::None,
            ResponseMode::ErrorPrefix,
        ),
    )
    .await
    .expect("response in time")
    .expect("response");

    assert!(
        matches!(&response.body, ResponseBody::Buffered(body) if body == b"ok"),
        "{:?}",
        response.body
    );
    let trailers = response.trailers.expect("trailers kept");
    assert_eq!(trailers[0].0.as_str(), "grpc-status");
    drop(server);
}

#[tokio::test]
async fn an_error_prefix_error_streams_before_the_body_ends() {
    let (client_io, mut peer) = tokio::io::duplex(1 << 16);
    let server = tokio::spawn(async move {
        let stream_id = request_stream(&mut peer).await;
        write_raw_headers(&mut peer, stream_id, &[(":status", "500")], false).await;
        write_data(&mut peer, stream_id, b"partial", false).await;
        tokio::time::sleep(Duration::from_secs(30)).await;
        peer
    });
    let handle = client(client_io).await;

    let response = timeout(
        Duration::from_secs(2),
        handle.send_shared(
            Arc::new(get_head("/")),
            RequestBody::None,
            ResponseMode::ErrorPrefix,
        ),
    )
    .await
    .expect("the head arrives before the body ends")
    .expect("response");

    assert_eq!(response.status, 500);
    let ResponseBody::Streaming(mut body) = response.body else {
        panic!("an error response must stream");
    };
    let first = timeout(Duration::from_secs(2), body.recv())
        .await
        .expect("first chunk in time")
        .expect("a chunk")
        .expect("chunk ok");
    assert_eq!(&first[..], b"partial");
    server.abort();
}
