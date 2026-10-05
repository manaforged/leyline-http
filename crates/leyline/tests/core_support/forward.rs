use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::net::TcpListener;

pub async fn counting_forwarder(target: std::net::SocketAddr) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&accepted);
    tokio::spawn(async move {
        while let Ok((mut inbound, _)) = listener.accept().await {
            count.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut outbound = tokio::net::TcpStream::connect(target).await.unwrap();
                drop(tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await);
            });
        }
    });
    (port, accepted)
}
