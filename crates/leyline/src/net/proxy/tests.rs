use super::*;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use crate::net::resolver::{ResolveFuture, Resolver};
use crate::profile::Platform;

struct RecordingResolver {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Resolver for RecordingResolver {
    fn resolve<'a>(&'a self, host: &'a str, _port: u16) -> ResolveFuture<'a> {
        self.seen.lock().unwrap().push(host.to_string());
        let addr = self.addr;
        Box::pin(async move { Ok(vec![addr]) })
    }
}

fn base_dialer() -> Dialer {
    Dialer::new(Platform::Linux.tcp_profile())
}

#[tokio::test]
async fn connect_to_proxy_routes_through_connector_resolver() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let dialer = base_dialer().with_resolver(Arc::new(RecordingResolver {
        addr,
        seen: seen.clone(),
    }));
    let url: url::Url = "http://proxy.test.invalid:8080".parse().unwrap();
    let stream = connect_to_proxy(&dialer, &url)
        .await
        .expect("dial reaches listener");
    assert_eq!(stream.peer_addr().unwrap().port(), addr.port());
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["proxy.test.invalid"],
        "proxy host must be resolved via the connector's resolver"
    );
}

#[tokio::test]
async fn connect_to_proxy_requires_host() {
    let url: url::Url = "mailto:a@b".parse().unwrap();
    connect_to_proxy(&base_dialer(), &url)
        .await
        .expect_err("expected Err");
}
