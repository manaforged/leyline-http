use leyline::cookie::Jar;
use leyline::{Error, Proxy, Response, Session, SessionBuilder};

const fn send_sync<T: Send + Sync + 'static>() {}

const fn send<T: Send>(_: &T) {}

#[test]
fn the_public_types_and_the_request_future_cross_threads() {
    send_sync::<Session>();
    send_sync::<SessionBuilder>();
    send_sync::<Response>();
    send_sync::<Error>();
    send_sync::<Jar>();
    send_sync::<Proxy>();
    let session = Session::new();
    send(&session.get("http://127.0.0.1:9/"));
    send(&session.get("http://127.0.0.1:9/").send());
}
