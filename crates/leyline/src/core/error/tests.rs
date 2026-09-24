use super::{Error, Kind};
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;
use crate::tls::TlsError;

fn io(kind: std::io::ErrorKind) -> Error {
    Error::new(Kind::Io).with_source(std::io::Error::new(kind, "io"))
}

fn tls(err: TlsError) -> Error {
    Error::new(Kind::Tls).with_source(err)
}

#[test]
fn timeout_kind_is_timeout_not_connect() {
    let err = Error::new(Kind::Timeout);
    assert!(err.is_timeout());
    assert!(!err.is_connect());
    assert!(!err.is_status());
    assert_eq!(err.kind(), Kind::Timeout);
}

#[test]
fn timed_out_io_is_timeout_not_connect() {
    let err = io(std::io::ErrorKind::TimedOut);
    assert!(err.is_timeout());
    assert!(!err.is_connect());
}

#[test]
fn body_io_is_not_connect() {
    let err = io(std::io::ErrorKind::UnexpectedEof);
    assert!(!err.is_connect());
    assert!(!err.is_timeout());
    assert!(err.is_connection_closed());
}

#[test]
fn file_io_is_not_connect() {
    assert!(!io(std::io::ErrorKind::NotFound).is_connect());
}

#[test]
fn refused_io_is_connect() {
    let err = io(std::io::ErrorKind::ConnectionRefused);
    assert!(err.is_connect());
    assert!(!err.is_timeout());
}

#[test]
fn connect_kind_is_connect() {
    assert!(Error::new(Kind::Connect).is_connect());
}

#[test]
fn tls_handshake_is_connect_and_connection_closed() {
    let err = tls(TlsError::Handshake("alert".into()));
    assert!(err.is_connect());
    assert!(err.is_connection_closed());
}

#[test]
fn cert_mismatch_is_not_connect() {
    let err = tls(TlsError::Hostname("wrong host".into()));
    assert!(!err.is_connect());
    assert!(!err.is_connection_closed());
}

#[test]
fn dns_is_connect_not_connection_closed() {
    let err = tls(TlsError::Dns(std::io::Error::other("nxdomain")));
    assert!(err.is_connect());
    assert!(!err.is_connection_closed());
}

#[test]
fn tls_io_timeout_is_timeout() {
    let err = tls(TlsError::TcpConnect(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "slow",
    )));
    assert!(err.is_timeout());
    assert!(err.is_connect());
}

#[test]
fn h2_goaway_and_refused_stream_are_connection_closed() {
    let goaway = Error::new(Kind::Http2).with_source(H2Error::Connection {
        code: ErrorCode::NoError,
        reason: String::new(),
    });
    assert!(goaway.is_connection_closed());
    let refused = Error::new(Kind::Http2).with_source(H2Error::Stream {
        stream_id: 1,
        code: ErrorCode::RefusedStream,
    });
    assert!(refused.is_connection_closed());
}

#[test]
fn status_kind_carries_status() {
    let err = Error::new(Kind::Status).with_status(http::StatusCode::FORBIDDEN);
    assert!(err.is_status());
    assert_eq!(err.status().map(|s| s.as_u16()), Some(403));
    assert!(!err.is_timeout());
    assert!(!err.is_connect());
}

#[test]
fn predicates_are_kind_scoped() {
    for kind in [Kind::Builder, Kind::Request, Kind::Config, Kind::Proxy] {
        let err = Error::new(kind);
        assert!(!err.is_timeout(), "{kind} must not be a timeout");
        assert!(!err.is_connect(), "{kind} must not be a connect failure");
        assert!(!err.is_status(), "{kind} must not carry a status");
    }
}

#[test]
fn source_returns_the_wrapped_error() {
    use std::error::Error as _;
    let connect = tls(TlsError::TcpConnect(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "refused",
    )));
    assert!(connect.source().is_some());
    assert!(connect.tls().is_some());

    let plain = io(std::io::ErrorKind::BrokenPipe);
    assert!(plain.source().is_some());
    assert!(plain.io().is_some());

    let url = Error::from_url_parse(url::Url::parse("::").expect_err("not a url"));
    assert_eq!(url.kind(), Kind::Url);
    assert!(url.source().is_some());

    let json = Error::from_json(serde_json::from_str::<u32>("nope").expect_err("not json"));
    assert_eq!(json.kind(), Kind::Json);
    assert!(json.source().is_some());

    assert!(Error::new(Kind::Timeout).source().is_none());
}

#[test]
fn display_names_the_kind_and_appends_the_url() {
    let err = Error::new(Kind::Config).with_message("no host in URL");
    assert_eq!(err.to_string(), "config: no host in URL");

    let with_url = Error::new(Kind::Status)
        .with_status(http::StatusCode::NOT_FOUND)
        .with_url("https://example.test/a".parse().expect("uri"));
    assert_eq!(
        with_url.to_string(),
        "status 404 for https://example.test/a"
    );

    let sourced = tls(TlsError::Hostname("wrong host".into()));
    assert_eq!(sourced.to_string(), "tls: hostname: wrong host");
}

#[test]
fn without_url_drops_the_url_from_display() {
    let err = Error::new(Kind::Request)
        .with_message("boom")
        .with_url("https://example.test/a".parse().expect("uri"));
    assert!(err.to_string().contains("example.test"));
    let bare = err.without_url();
    assert!(bare.url().is_none());
    assert_eq!(bare.to_string(), "http: boom");
}

#[test]
fn debug_redacts_userinfo() {
    let err = Error::new(Kind::Request)
        .with_url("https://user:pass@example.test/a".parse().expect("uri"));
    let text = format!("{err:?}");
    assert!(!text.contains("user:pass"), "userinfo leaked: {text}");
    assert!(
        text.contains("***@example.test"),
        "unexpected debug: {text}"
    );
    assert!(
        !err.to_string().contains("pass"),
        "userinfo leaked in Display"
    );
}
