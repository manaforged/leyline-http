//! Compile-time pins on the public request and response shapes built from the `http` crate's types.

#![allow(dead_code)]

use std::time::Duration;

use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use leyline::profile::ProfileError;
use leyline::{
    BrowserProfile, CompressionConfig, Error, PoolConfig, RedirectAction, RedirectPolicy, Request,
    Response, RetryPolicy, Session, SocketConfig, TimeoutConfig, WebSocketConfig,
};
#[cfg(feature = "websocket")]
use leyline::{CloseFrame, WsMessage};

/// Each binding pins one accessor's exact public shape.
fn _response_header_api_is_pinned(r: &Response) {
    let _status: StatusCode = r.status();

    let _headers: std::option::Option<(&HeaderName, &HeaderValue)> = r.headers().next();
    let _trailers: std::option::Option<(&HeaderName, &HeaderValue)> = r.trailers().next();
    let _map: HeaderMap = r.header_map();

    let _first: std::option::Option<&str> = r.header("content-type");
    let _all: std::option::Option<&str> = r.header_all("set-cookie").next();
    let _ct: std::option::Option<&str> = r.content_type();
    let _len: std::option::Option<u64> = r.content_length();

    let _cookies: std::option::Option<(&str, &str)> = r.cookies().next();
    let _cookie: std::option::Option<&str> = r.cookie("sid");

    let _req: std::option::Option<(&str, &str)> = r.request_headers().next();
}

/// The owned request carries `http` types; the session takes a `Method` and anything that parses as a `Uri`.
fn _request_api_is_pinned(session: &Session) {
    let mut req = Request::new(Method::POST, "https://example.test/");
    let _method: &Method = &req.method;
    let _url: &Uri = &req.url;
    req = req.header("x-a", "1");
    drop(req);

    drop(session.request(Method::GET, "https://example.test/"));
    drop(session.get("https://example.test/").header("x-a", "1"));
}

/// A status error carries a `StatusCode`.
fn _error_status_is_pinned(err: &Error) {
    let _code: std::option::Option<StatusCode> = err.status();
}

/// Config structs are `#[non_exhaustive]`; setters chained off `default()` are the only way to build one.
fn _config_builders_are_pinned() {
    let _timeouts: TimeoutConfig = TimeoutConfig::default()
        .total(Duration::from_secs(5))
        .connect(Duration::from_secs(1))
        .read(None)
        .response_header(Duration::from_secs(2));
    let _pool: PoolConfig = PoolConfig::default().max_connections(4).keepalive(true);
    let _socket: SocketConfig = SocketConfig::default().tcp_nodelay(true).strict(false);
    let _compression: CompressionConfig = CompressionConfig::default().gzip(false);
    let _ws: WebSocketConfig = WebSocketConfig::default().max_frame_size(4096);
    let _retry: RetryPolicy = RetryPolicy::transient().backoff_factor(1.5).jitter(false);
}

/// A redirect attempt reports the current URL as an `http::Uri`.
fn _redirect_attempt_is_pinned() -> RedirectPolicy {
    RedirectPolicy::custom(|attempt| {
        let _url: &Uri = attempt.url;
        RedirectAction::Follow
    })
}

/// `WsMessage` is a Leyline enum, not the wire library's message type.
#[cfg(feature = "websocket")]
fn _ws_message_is_pinned(msg: WsMessage) -> std::option::Option<u16> {
    match msg {
        WsMessage::Text(s) => u16::try_from(s.len()).ok(),
        WsMessage::Binary(b) => u16::try_from(b.len()).ok(),
        WsMessage::Ping | WsMessage::Pong => None,
        WsMessage::Close(frame) => frame.map(|f: CloseFrame| f.code),
        _ => None,
    }
}

/// Profile parsing reports `ProfileError`; the `toml` error type stays off the public API.
fn _profile_parse_error_is_pinned(text: &str) {
    let parsed: std::result::Result<BrowserProfile, ProfileError> = BrowserProfile::from_toml(text);
    drop(parsed);
}

#[test]
fn public_header_api_compiles_in_pinned_shape() {
    fn _takes_fn(_: fn(&Response)) {}
    _takes_fn(_response_header_api_is_pinned);
}
