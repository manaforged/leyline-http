use leyline::Session;
use leyline::testing::{Response, Server};

#[tokio::test]
async fn a_session_built_after_sslkeylogfile_is_set_logs_its_keys() {
    let server = Server::https(|_| Response::new(200)).await.unwrap();
    let url = format!("https://localhost:{}/", server.addr().port());
    let first = Session::builder().tls(server.tls()).build().unwrap();
    first.get(&url).send().await.unwrap();

    let path = std::env::temp_dir().join(format!("leyline-keylog-{}.txt", std::process::id()));
    // SAFETY: this binary holds one test, so no other thread reads the environment.
    unsafe { std::env::set_var("SSLKEYLOGFILE", &path) };
    let second = Session::builder().tls(server.tls()).build().unwrap();
    second.get(&url).send().await.unwrap();
    // SAFETY: as above, no other thread reads the environment.
    unsafe { std::env::remove_var("SSLKEYLOGFILE") };

    let logged = std::fs::read_to_string(&path).unwrap_or_default();
    drop(std::fs::remove_file(&path));
    assert!(
        logged.contains("CLIENT_"),
        "no TLS secrets logged: {logged:?}"
    );
}
