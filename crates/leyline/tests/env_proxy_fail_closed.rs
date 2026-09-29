use leyline::{Kind, Session};

#[tokio::test]
async fn an_invalid_environment_proxy_fails_closed() {
    // SAFETY: this binary holds one test on a current-thread runtime, so nothing reads the environment while it changes.
    unsafe {
        std::env::set_var("HTTPS_PROXY", "http://[::1");
        std::env::set_var("HTTP_PROXY", "http://[::1");
    }
    let err = Session::builder().build().unwrap_err();
    assert_eq!(err.kind(), Kind::Config, "{err:?}");
    let err = Session::new().get("http://127.0.0.1:9/").await.unwrap_err();
    assert_eq!(err.kind(), Kind::Proxy, "{err:?}");
}
