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
    let (_, headers, _) = parse_h1_head(head).unwrap();
    let set_cookies: Vec<&str> = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
        .map(|(_, v)| v.as_str())
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
    assert!(headers.iter().any(|(k, v)| k == "X-Real" && v == "ok"));
}

#[test]
fn fold_without_previous_header_is_dropped() {
    let head = "HTTP/1.1 200 OK\r\n\tContent-Length: 999\r\n\r\n";
    let (_, headers, _) = parse_h1_head(head).unwrap();
    assert!(headers.is_empty(), "{headers:?}");
}

#[test]
fn whitespace_before_colon_drops_the_line() {
    let head = "HTTP/1.1 200 OK\r\n\
                    Transfer-Encoding : chunked\r\n\
                    X-Ok: 1\r\n\
                    \r\n";
    let (_, headers, _) = parse_h1_head(head).unwrap();
    assert!(
        !headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("transfer-encoding"))
    );
    assert!(!headers.iter().any(|(k, _)| k.contains(' ')));
    assert!(headers.iter().any(|(k, _)| k == "X-Ok"));
}

#[test]
fn empty_header_name_dropped() {
    let head = "HTTP/1.1 200 OK\r\n: value\r\nX-Real: ok\r\n\r\n";
    let (_, headers, _) = parse_h1_head(head).unwrap();
    assert_eq!(headers.len(), 1, "{headers:?}");
    assert_eq!(headers[0].0, "X-Real");
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
    assert!(conn_is_live(&mut client as &mut dyn H1Io));
}

#[tokio::test]
async fn peer_closed_socket_probes_as_dead() {
    let (mut client, server) = tcp_pair().await;
    drop(server);
    client.readable().await.unwrap();
    assert!(!conn_is_live(&mut client as &mut dyn H1Io));
}

#[tokio::test]
async fn socket_with_pending_bytes_probes_as_dead() {
    let (mut client, mut server) = tcp_pair().await;
    server.write_all(b"x").await.unwrap();
    server.flush().await.unwrap();
    client.readable().await.unwrap();
    assert!(!conn_is_live(&mut client as &mut dyn H1Io));
}

#[tokio::test]
async fn checkout_live_h1_drains_dead_and_counts_stale() {
    let pool = Arc::new(Pool::new());
    let key = make_key("http", "127.0.0.1", 1, None, Transport::Tcp);

    let (client, server) = tcp_pair().await;
    drop(server);
    client.readable().await.unwrap();
    pool.return_h1(
        key.clone(),
        H1Slot {
            io: Box::new(client),
        },
        TlsInfo::default(),
    );

    assert!(checkout_live_h1(&pool, &key).is_none());
    let stats = pool.stats();
    assert_eq!(stats.stale_probed, 1, "probe catch must count as stale");
    assert_eq!(stats.evictions_dead, 0, "no mid-exchange failure occurred");
}

#[tokio::test]
async fn checkout_live_h1_returns_a_live_connection_uncounted() {
    let pool = Arc::new(Pool::new());
    let key = make_key("http", "127.0.0.1", 2, None, Transport::Tcp);

    let (client, _server) = tcp_pair().await;
    pool.return_h1(
        key.clone(),
        H1Slot {
            io: Box::new(client),
        },
        TlsInfo::default(),
    );

    assert!(
        checkout_live_h1(&pool, &key).is_some(),
        "a live pooled connection must be handed out"
    );
    assert_eq!(
        pool.stats().stale_probed,
        0,
        "a live connection is not a probe catch"
    );
}
