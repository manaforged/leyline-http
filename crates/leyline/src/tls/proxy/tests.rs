use super::*;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use crate::profile::{Browser, ProfileRegistry};
use crate::tcp::TcpProfile;
use crate::tls::{FingerprintConnector, ResolveFuture, Resolver};

/// Resolver that records every host it is asked to resolve and always
/// returns one fixed loopback address — so a test can prove the proxy dial
/// is routed through the connector's resolver (no DNS leak) and lands on a
/// known local listener.
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

fn base_connector() -> FingerprintConnector {
    let reg = ProfileRegistry::builtin();
    let profile = reg
        .get_browser(Browser::default_browser())
        .expect("default profile present");
    FingerprintConnector::new(profile, TcpProfile::LINUX).expect("connector build")
}

// The proxy TCP leg must be dialed through the connector's pluggable
// resolver, not a bare `TcpStream::connect`. Otherwise a custom/DoH
// resolver is bypassed for the proxy hostname (DNS leak vs. caller intent)
// and the SYN carries a non-browser TCP fingerprint. This asserts the
// connector's resolver is the one consulted for the proxy host.
#[tokio::test]
async fn connect_to_proxy_routes_through_connector_resolver() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let connector = base_connector().with_resolver(Arc::new(RecordingResolver {
        addr,
        seen: seen.clone(),
    }));
    let url: url::Url = "http://proxy.test.invalid:8080".parse().unwrap();
    let stream = connect_to_proxy(&connector, &url, 8080)
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
    // Cannot-be-a-base URLs parse with no host component.
    let url: url::Url = "mailto:a@b".parse().unwrap();
    assert!(
        connect_to_proxy(&base_connector(), &url, 8080)
            .await
            .is_err()
    );
}
