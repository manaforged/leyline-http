#![cfg(not(feature = "socks"))]

use leyline::Session;
use leyline::testing::{TestResponse, TestServer};

#[tokio::test]
async fn a_socks_environment_proxy_leaves_no_proxy_hosts_working() {
    // SAFETY: this binary holds one test on a current-thread runtime, so nothing reads the environment while it changes.
    unsafe {
        std::env::set_var("ALL_PROXY", "socks5://127.0.0.1:1");
        std::env::set_var("NO_PROXY", "127.0.0.1");
    }
    let server = TestServer::http(|_| TestResponse::new(200)).await.unwrap();
    let session = Session::builder().build().unwrap();
    let resp = session.get(server.url("/")).await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
}
