use super::*;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

#[test]
fn obs_fold_appends_to_previous_header_never_creates_one() {
    let head = "HTTP/1.1 200 OK\r\n\
                    Set-Cookie: a=1\r\n\
                    \tSet-Cookie: evil=x; Domain=.example.com\r\n\
                    X-Real: ok\r\n\
                    \r\n";
    let (_, headers, _) = parse_h1_head(head.as_bytes()).unwrap();
    let set_cookies: Vec<&str> = headers
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap())
        .collect();
    assert_eq!(
        set_cookies.len(),
        1,
        "fold must never create a second header"
    );
    assert_eq!(
        set_cookies[0],
        "a=1 Set-Cookie: evil=x; Domain=.example.com"
    );
    assert_eq!(headers.get("x-real").unwrap(), "ok");
}

#[test]
fn fold_without_previous_header_is_dropped() {
    let head = "HTTP/1.1 200 OK\r\n\tContent-Length: 999\r\n\r\n";
    let (_, headers, _) = parse_h1_head(head.as_bytes()).unwrap();
    assert!(headers.is_empty(), "{headers:?}");
}

#[test]
fn whitespace_before_colon_drops_the_line() {
    let head = "HTTP/1.1 200 OK\r\n\
                    Transfer-Encoding : chunked\r\n\
                    X-Ok: 1\r\n\
                    \r\n";
    parse_h1_head(head.as_bytes()).expect_err("expected Err");
}

#[test]
fn empty_header_name_dropped() {
    let head = "HTTP/1.1 200 OK\r\n: value\r\nX-Real: ok\r\n\r\n";
    parse_h1_head(head.as_bytes()).expect_err("expected Err");
}

async fn tcp_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client = TcpStream::connect(addr).await.unwrap();
    let (server, _) = listener.accept().await.unwrap();
    (client, server)
}

#[tokio::test]
async fn live_idle_socket_probes_as_live() {
    let (mut client, _server) = tcp_pair().await;
    assert!(conn_is_live(&mut client as &mut dyn H1Io, true));
}

#[tokio::test]
async fn peer_closed_socket_probes_as_dead() {
    let (mut client, server) = tcp_pair().await;
    drop(server);
    client.readable().await.unwrap();
    assert!(!conn_is_live(&mut client as &mut dyn H1Io, true));
}

#[tokio::test]
async fn socket_with_pending_bytes_probes_as_dead() {
    let (mut client, mut server) = tcp_pair().await;
    server.write_all(b"x").await.unwrap();
    server.flush().await.unwrap();
    client.readable().await.unwrap();
    assert!(!conn_is_live(&mut client as &mut dyn H1Io, true));
}

#[tokio::test]
async fn checkout_live_h1_drains_dead_and_counts_stale() {
    let pool = Arc::new(Pool::new());
    let key = pool.key("http", "127.0.0.1", 1, None, Transport::Tcp);

    let (client, server) = tcp_pair().await;
    drop(server);
    client.readable().await.unwrap();
    pool.return_h1(&key, H1Slot::new(Box::new(client)), TlsInfo::default());

    assert!(checkout_live_h1(&pool, &key, true, std::time::Instant::now()).is_none());
    let stats = pool.stats();
    assert_eq!(stats.stale_probed, 1, "probe catch must count as stale");
    assert_eq!(stats.evictions_dead, 0, "no mid-exchange failure occurred");
}

#[tokio::test]
async fn checkout_live_h1_returns_a_live_connection_uncounted() {
    let pool = Arc::new(Pool::new());
    let key = pool.key("http", "127.0.0.1", 2, None, Transport::Tcp);

    let (client, _server) = tcp_pair().await;
    pool.return_h1(&key, H1Slot::new(Box::new(client)), TlsInfo::default());

    assert!(
        checkout_live_h1(&pool, &key, true, std::time::Instant::now()).is_some(),
        "a live pooled connection must be handed out"
    );
    assert_eq!(
        pool.stats().stale_probed,
        0,
        "a live connection is not a probe catch"
    );
}

#[test]
fn a_chunk_size_must_be_hex_digits_only() {
    assert_eq!(parse::chunk_size(b"1a;ext=1"), Ok(26));
    for bad in [&b"+5"[..], b"-0", b"", b"0x5", b" "] {
        parse::chunk_size(bad).unwrap_err();
    }
}

#[tokio::test]
async fn warm_send_future_leaves_the_dial_frame_on_the_heap() {
    let pool = Arc::new(Pool::new());
    let profile = crate::profile::BrowserProfile::bare_shared();
    let connector =
        FingerprintConnector::new(&profile, crate::profile::Platform::Linux.tcp_profile())
            .expect("connector build");
    let url = url::Url::parse("http://example.com/").expect("url");
    let key = pool.key("http", "example.com", 80, None, Transport::Tcp);
    let request = H1Request {
        scheme: "http",
        host: "example.com",
        port: 80,
        method: "GET",
        url: &url,
        headers: Vec::new().into(),
        proxy: None,
        target: crate::pool::H1Target::OriginForm,
        response: ResponseMode::Buffered,
    };
    let send = send_h1_pooled(
        &pool,
        &connector,
        request,
        Body::default(),
        None,
        H1Dial::Http1Only,
    );
    let dial = fresh_leg(&pool, &key, &connector, H1Dial::Http1Only, None);
    assert!(std::mem::size_of_val(&send) < std::mem::size_of_val(&dial));
}

#[tokio::test]
async fn a_buffered_body_leaves_the_read_buffer_with_the_connection() {
    let mut initial_body = Vec::with_capacity(4096);
    initial_body.extend_from_slice(b"ok-10byte!");
    let mut head = H1Head {
        status: 200,
        headers: crate::core::response_head::Fields::default(),
        minor: 1,
        framing: BodyFraming::Fixed(10),
        initial_body,
    };
    let (mut client, _server) = tcp_pair().await;
    let (body, excess) = read_h1_body(&mut client, &mut head, usize::MAX)
        .await
        .expect("buffered body");
    assert_eq!(body, b"ok-10byte!");
    assert!(!excess);
    assert!(head.initial_body.is_empty());
    assert!(head.initial_body.capacity() >= 4096);
}

#[test]
fn a_header_value_allows_tab_visible_ascii_and_utf8_only() {
    for byte in 0u8..0x80 {
        let value = char::from(byte).to_string();
        let allowed = byte == b'\t' || (b' '..=b'~').contains(&byte);
        assert_eq!(
            headers::is_valid_header_value(&value),
            allowed,
            "byte {byte:#04x}"
        );
    }
    assert!(headers::is_valid_header_value("café au lait"));
    assert!(!headers::is_valid_header_value("ok\r\nInjected: yes"));
}

#[tokio::test]
async fn time_queued_for_a_connection_is_not_counted_as_request_time() {
    use crate::testing::{Response, Server};
    let server = Server::http(|_| {
        Response::new(200)
            .delay(Duration::from_millis(250))
            .body("ok")
    })
    .await
    .expect("server starts");
    let session = crate::Session::builder()
        .protocol(crate::Protocol::Http1)
        .max_connections_per_host(1)
        .build()
        .expect("session");

    let url = server.url("/");
    let (first, second) = tokio::join!(
        session.get(url.as_str()).send(),
        session.get(url.as_str()).send()
    );
    let (first, second) = (first.expect("first"), second.expect("second"));

    let queued = first.elapsed().max(second.elapsed());
    assert!(queued < Duration::from_millis(450), "elapsed {queued:?}");
}
