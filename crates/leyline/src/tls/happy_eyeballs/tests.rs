use super::*;
use std::net::Ipv6Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::net::TcpListener;
use tokio::time::Instant;

#[test]
fn interleave_v6_first() {
    let v6a: SocketAddr = "[fe80::1]:443".parse().unwrap();
    let v6b: SocketAddr = "[fe80::2]:443".parse().unwrap();
    let v4a: SocketAddr = "127.0.0.1:443".parse().unwrap();
    let v4b: SocketAddr = "127.0.0.2:443".parse().unwrap();

    let out = interleave_by_family(vec![v4a, v6a, v4b, v6b]);
    assert_eq!(out, vec![v6a, v4a, v6b, v4b]);
}

#[test]
fn interleave_imbalanced() {
    let v6a: SocketAddr = "[fe80::1]:443".parse().unwrap();
    let v4a: SocketAddr = "127.0.0.1:443".parse().unwrap();
    let v4b: SocketAddr = "127.0.0.2:443".parse().unwrap();
    let v4c: SocketAddr = "127.0.0.3:443".parse().unwrap();

    let out = interleave_by_family(vec![v4a, v4b, v6a, v4c]);
    assert_eq!(out, vec![v6a, v4a, v4b, v4c]);
}

async fn spawn_listener() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            if listener.accept().await.is_err() {
                break;
            }
        }
    });
    addr
}

#[tokio::test]
async fn single_address_succeeds() {
    let addr = spawn_listener().await;
    let (_stream, winner) = happy_eyeballs_connect(
        vec![addr],
        HappyEyeballsConfig::default(),
        |sa| async move { TcpStream::connect(sa).await },
    )
    .await
    .expect("connect should succeed");
    assert_eq!(winner, addr);
}

#[tokio::test]
async fn v6_failure_falls_through_to_v4() {
    let v4_addr = spawn_listener().await;
    let v6_unreachable: SocketAddr = SocketAddr::from((
        Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0xdead, 0xbeef),
        v4_addr.port(),
    ));

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_c = attempts.clone();

    let config = HappyEyeballsConfig {
        resolve_delay: Duration::from_millis(50),
        attempt_limit: 8,
    };

    let start = Instant::now();
    let (_stream, winner) =
        happy_eyeballs_connect(vec![v6_unreachable, v4_addr], config, move |sa| {
            let attempts = attempts_c.clone();
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                TcpStream::connect(sa).await
            }
        })
        .await
        .expect("should fall back to IPv4");
    let elapsed = start.elapsed();

    assert_eq!(winner, v4_addr);
    assert!(attempts.load(Ordering::SeqCst) >= 1);
    assert!(
        elapsed < Duration::from_secs(5),
        "fallback too slow: {elapsed:?}"
    );
}

#[tokio::test]
async fn all_failures_surface_last_error() {
    let port1 = {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = l.local_addr().unwrap().port();
        drop(l);
        p
    };
    let port2 = {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = l.local_addr().unwrap().port();
        drop(l);
        p
    };
    let a: SocketAddr = format!("127.0.0.1:{port1}").parse().unwrap();
    let b: SocketAddr = format!("127.0.0.1:{port2}").parse().unwrap();

    let err = happy_eyeballs_connect(
        vec![a, b],
        HappyEyeballsConfig {
            resolve_delay: Duration::from_millis(10),
            attempt_limit: 8,
        },
        |sa| async move { TcpStream::connect(sa).await },
    )
    .await
    .expect_err("both should fail");
    assert!(
        matches!(
            err.kind(),
            io::ErrorKind::ConnectionRefused | io::ErrorKind::AddrNotAvailable
        ),
        "unexpected error kind: {err:?}"
    );
}

#[tokio::test]
async fn empty_input_errs() {
    let err = happy_eyeballs_connect(
        Vec::new(),
        HappyEyeballsConfig::default(),
        |sa| async move { TcpStream::connect(sa).await },
    )
    .await
    .expect_err("empty addr list must error");
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
}
