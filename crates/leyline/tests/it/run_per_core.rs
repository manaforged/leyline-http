use std::rc::Rc;

use leyline::Session;
use leyline::testing::{Response, Server};

#[tokio::test(flavor = "multi_thread")]
async fn every_core_fetches_with_its_own_session() {
    let server = Server::http(|_| Response::new(200).body("ok"))
        .await
        .expect("server starts");
    let url = format!("http://{}/", server.addr());
    let bodies = tokio::task::spawn_blocking(move || {
        Session::builder().run_per_core(|session| {
            let url = Rc::new(url.clone());
            async move { session.get(url.as_str()).send().await?.text().await }
        })
    })
    .await
    .expect("runner thread")
    .expect("every session builds");
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    assert_eq!(bodies.len(), cores);
    for body in bodies {
        assert_eq!(body.expect("request succeeds"), "ok");
    }
}

#[test]
fn a_builder_error_is_returned_before_any_work_runs() {
    let ran = std::sync::atomic::AtomicBool::new(false);
    let err = Session::builder()
        .proxy("not a proxy url")
        .run_per_core(|_| async { ran.store(true, std::sync::atomic::Ordering::Relaxed) })
        .unwrap_err();
    assert!(
        err.is_proxy() || err.kind() == leyline::Kind::Config,
        "{err:?}"
    );
    assert!(!ran.load(std::sync::atomic::Ordering::Relaxed));
}
