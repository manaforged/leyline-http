use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use leyline::testing::{Response, Server};
use leyline::{Body, HostLimits, Kind, Session};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::v030_workloads::TempDir;

#[tokio::test]
async fn pool_max_lifetime_evicts_aged_idle_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepts = Arc::new(AtomicUsize::new(0));
    let accepts_bg = Arc::clone(&accepts);
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            accepts_bg.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                loop {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            if sock
                                .write_all(
                                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\nok",
                                )
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                }
            });
        }
    });

    let url = format!("http://{addr}/life");
    let session = Session::builder()
        .pool_max_lifetime(Duration::from_millis(80))
        .build()
        .unwrap();
    assert_eq!(session.get(&url).text().await.unwrap(), "ok");
    assert_eq!(session.get(&url).text().await.unwrap(), "ok");
    assert_eq!(
        accepts.load(Ordering::SeqCst),
        1,
        "fresh connection within pool_max_lifetime must be reused"
    );

    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(session.get(&url).text().await.unwrap(), "ok");
    assert_eq!(
        accepts.load(Ordering::SeqCst),
        2,
        "connection older than pool_max_lifetime must be evicted and redialed"
    );
}

#[tokio::test]
async fn host_limits_max_pending_rejects_excess_waiters_with_overloaded() {
    let server = Server::http(|_| {
        Response::new(200)
            .delay(Duration::from_millis(120))
            .body("ok")
    })
    .await
    .unwrap();

    let session = Session::builder()
        .host_limits(HostLimits::new().max_in_flight(1).max_pending(1))
        .retry(0)
        .build()
        .unwrap();

    let mut handles = Vec::new();
    for _ in 0..4 {
        let s = session.clone();
        let url = server.url("/slow");
        handles.push(tokio::spawn(async move { s.get(url).send().await }));
    }

    let mut ok = 0usize;
    let mut overloaded = 0usize;
    for handle in handles {
        match handle.await.unwrap() {
            Ok(resp) if resp.status().as_u16() == 200 => ok += 1,
            Err(err) if err.kind() == Kind::Overloaded => overloaded += 1,
            other => panic!("unexpected result under max_pending(1): {other:?}"),
        }
    }
    assert_eq!(ok, 2, "1 active + 1 pending waiter must succeed");
    assert_eq!(
        overloaded, 2,
        "excess callers beyond max_pending(1) must fail fast with Kind::Overloaded"
    );
}

#[tokio::test]
async fn socket_tuning_keepalive_retries_send_buffer_and_strict_options() {
    let server = Server::http(|_| Response::new(200).body("tuned"))
        .await
        .unwrap();
    let session = Session::builder()
        .tcp_keepalive(Duration::from_secs(10))
        .tcp_keepalive_interval(Duration::from_secs(5))
        .tcp_keepalive_retries(3)
        .send_buffer_size(64 * 1024)
        .strict_socket_options(true)
        .build()
        .unwrap();
    assert_eq!(session.get(server.url("/")).text().await.unwrap(), "tuned");
}

#[cfg(feature = "websocket")]
#[tokio::test]
async fn all_websocket_tuning_options_enforced_on_wire() {
    use futures_util::SinkExt;
    use leyline::Message;
    use leyline_bssl::ssl::{Ssl, SslContextBuilder, SslMethod};

    let (cert, key) = super::tls_support::self_signed();
    let ca_der = cert.to_der().unwrap();
    let mut ctx = SslContextBuilder::new(SslMethod::tls()).unwrap();
    ctx.set_certificate(&cert).unwrap();
    ctx.set_private_key(&key).unwrap();
    let ctx = Arc::new(ctx.build());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((tcp, _)) = listener.accept().await {
            let ssl = Ssl::new(&ctx).unwrap();
            let tls_stream = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                .accept()
                .await
                .unwrap();
            let mut ws = tokio_tungstenite::accept_async(tls_stream).await.unwrap();
            if let Some(Ok(msg)) = ws.next().await {
                let text = msg.into_text().unwrap();
                if text == "small" {
                    ws.send(tokio_tungstenite::tungstenite::Message::Text(
                        "ok-small".into(),
                    ))
                    .await
                    .unwrap();
                }
            }
            if let Some(Ok(_)) = ws.next().await {
                let big = "x".repeat(128);
                drop(
                    ws.send(tokio_tungstenite::tungstenite::Message::Text(big.into()))
                        .await,
                );
            }
        }
    });

    let session = Session::builder()
        .tls(
            leyline::Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_der(ca_der),
        )
        .websocket_http2(false)
        .websocket_max_frame_size(32)
        .websocket_max_message_size(32)
        .websocket_read_buffer_size(4096)
        .websocket_write_buffer_size(4096)
        .websocket_max_write_buffer_size(16 * 1024)
        .websocket_accept_unmasked_frames(false)
        .build()
        .unwrap();

    let ws = session
        .get(format!("wss://{addr}/tuned"))
        .websocket()
        .await
        .unwrap();
    let (mut sink, mut stream) = ws.split();
    sink.send(Message::Text("small".into())).await.unwrap();
    assert_eq!(
        stream.next().await.unwrap().unwrap(),
        Message::Text("ok-small".into())
    );

    sink.send(Message::Text("trigger-big".into()))
        .await
        .unwrap();
    let err = stream.next().await.unwrap().unwrap_err();
    assert_ne!(
        err.kind(),
        Kind::Config,
        "128-byte frame must fail under websocket_max_frame_size(32): {err:?}"
    );
}

#[tokio::test]
async fn response_trailers_parsed_on_h1_chunked_and_h2_trailing_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let h1_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut sock, _)) = listener.accept().await {
            let mut req = [0u8; 1024];
            drop(sock.read(&mut req).await);
            let raw = b"HTTP/1.1 200 OK\r\n\
                        Transfer-Encoding: chunked\r\n\
                        Trailer: x-digest\r\n\
                        Connection: close\r\n\r\n\
                        5\r\nhello\r\n\
                        0\r\n\
                        x-digest: sha256-h1\r\n\r\n";
            drop(sock.write_all(raw).await);
        }
    });

    let session = Session::builder().build().unwrap();
    let resp = session
        .get(format!("http://{h1_addr}/chunked"))
        .send()
        .await
        .unwrap();
    let trailers: Vec<(String, String)> = resp
        .trailers()
        .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap().to_owned()))
        .collect();
    assert_eq!(
        trailers,
        vec![("x-digest".to_owned(), "sha256-h1".to_owned())]
    );
    assert_eq!(resp.text().await.unwrap(), "hello");

    #[cfg(feature = "bench-internals")]
    {
        use super::h2_support::{
            read_frame, read_preface, write_raw_headers, write_server_settings, write_settings_ack,
        };
        use super::tls_support::{H2, self_signed, tls_server_per_connection};
        use bytes::BytesMut;
        use leyline::internals::h2::frame::{DataFrame, FrameType};

        let (cert, key) = self_signed();
        let der = cert.to_der().unwrap();
        let port = tls_server_per_connection(
            cert,
            key,
            Arc::new(AtomicUsize::new(0)),
            |_| H2,
            |_, mut stream| async move {
                read_preface(&mut stream).await;
                write_server_settings(&mut stream).await;
                loop {
                    let (hdr, _) =
                        match tokio::time::timeout(Duration::from_secs(3), read_frame(&mut stream))
                            .await
                        {
                            Ok(f) => f,
                            Err(_) => break,
                        };
                    if hdr.frame_type == FrameType::Settings as u8 && hdr.flags & 0x1 == 0 {
                        write_settings_ack(&mut stream).await;
                    } else if hdr.frame_type == FrameType::Headers as u8 {
                        write_raw_headers(&mut stream, hdr.stream_id, &[(":status", "200")], false)
                            .await;
                        let mut buf = BytesMut::new();
                        DataFrame {
                            stream_id: hdr.stream_id,
                            end_stream: false,
                            data: bytes::Bytes::from_static(b"h2-body"),
                            wire_len: 7,
                        }
                        .encode(&mut buf);
                        stream.write_all(&buf).await.unwrap();
                        write_raw_headers(
                            &mut stream,
                            hdr.stream_id,
                            &[("x-h2-trailer", "verified")],
                            true,
                        )
                        .await;
                        break;
                    }
                }
            },
        )
        .await;

        let h2_session = Session::builder()
            .protocol(leyline::Protocol::Http2)
            .tls(
                leyline::Tls::new()
                    .system_roots(false)
                    .env_roots(false)
                    .ca_der(der),
            )
            .build()
            .unwrap();
        let resp = h2_session
            .get(format!("https://127.0.0.1:{port}/h2-trailers"))
            .send()
            .await
            .unwrap();
        let h2_trailers: Vec<(String, String)> = resp
            .trailers()
            .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap().to_owned()))
            .collect();
        assert_eq!(
            h2_trailers,
            vec![("x-h2-trailer".to_owned(), "verified".to_owned())]
        );
        assert_eq!(resp.text().await.unwrap(), "h2-body");
    }
}

#[tokio::test]
async fn pool_stats_record_idle_and_lru_evictions() {
    let origin_a = Server::http(|_| Response::new(200).body("a"))
        .await
        .unwrap();
    let origin_b = Server::http(|_| Response::new(200).body("b"))
        .await
        .unwrap();

    let idle_session = Session::builder()
        .pool_idle_timeout(Duration::from_millis(60))
        .build()
        .unwrap();
    assert_eq!(
        idle_session.get(origin_a.url("/")).text().await.unwrap(),
        "a"
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while idle_session.stats().evictions_idle() == 0 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        idle_session.stats().evictions_idle() >= 1,
        "idle reaper must increment evictions_idle: {:?}",
        idle_session.stats()
    );

    let lru_session = Session::builder().max_connections(1).build().unwrap();
    assert_eq!(
        lru_session.get(origin_a.url("/")).text().await.unwrap(),
        "a"
    );
    assert_eq!(
        lru_session.get(origin_b.url("/")).text().await.unwrap(),
        "b"
    );
    assert!(
        lru_session.stats().evictions_lru() >= 1,
        "exceeding max_connections(1) across origins must increment evictions_lru: {:?}",
        lru_session.stats()
    );
}

#[tokio::test]
async fn large_2mib_streaming_upload_download_and_body_cap() {
    let payload: Vec<u8> = (0..(2 * 1024 * 1024usize))
        .map(|i| ((i.wrapping_mul(131)) ^ (i >> 9)) as u8)
        .collect();
    let expected_sha: [u8; 32] = Sha256::digest(&payload).into();
    let download_arc = Arc::new(payload.clone());

    let server = Server::http(move |req| match req.target() {
        "/big-down" => Response::new(200).body(download_arc.as_ref().clone()),
        "/big-up" => {
            let sum: [u8; 32] = Sha256::digest(req.body()).into();
            Response::new(200).body(format!("{}:{:x}", req.body().len(), hex_digest(&sum)))
        }
        _ => Response::new(404),
    })
    .await
    .unwrap();

    let capped = Session::builder()
        .max_body_size(1024 * 1024)
        .build()
        .unwrap();
    let err = capped
        .get(server.url("/big-down"))
        .send()
        .await
        .unwrap_err();
    assert_eq!(
        err.kind(),
        Kind::BodyLimit,
        "2 MiB download must fail under 1 MiB max_body_size"
    );

    let uncapped = Session::builder().max_body_size(None).build().unwrap();
    let resp = uncapped
        .get(server.url("/big-down"))
        .stream()
        .send()
        .await
        .unwrap();
    let mut stream = resp.bytes_stream().unwrap();
    let mut hasher = Sha256::new();
    let mut total = 0usize;
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.unwrap();
        total += bytes.len();
        hasher.update(&bytes);
    }
    assert_eq!(total, 2 * 1024 * 1024);
    let actual_sha: [u8; 32] = hasher.finalize().into();
    assert_eq!(actual_sha, expected_sha);

    let dir = TempDir::new("large-2mib-upload");
    let up_file = dir.path("2mib.bin");
    std::fs::write(&up_file, &payload).unwrap();
    let body = Body::file(&up_file).unwrap();
    assert_eq!(body.content_length(), Some(2 * 1024 * 1024));
    let up_resp = uncapped
        .post(server.url("/big-up"))
        .body(body)
        .text()
        .await
        .unwrap();
    assert_eq!(
        up_resp,
        format!("{}:{:x}", 2 * 1024 * 1024, hex_digest(&expected_sha))
    );
}

struct HexDigest([u8; 32]);

fn hex_digest(bytes: &[u8; 32]) -> HexDigest {
    HexDigest(*bytes)
}

impl std::fmt::LowerHex for HexDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}
