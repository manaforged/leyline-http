use super::connect::connect_err;
use super::*;
use crate::h2::{ErrorCode, H2Error};
use crate::tls::TlsError;
use crate::{Cause, Error};

#[test]
fn transport_separates_the_keyspace() {
    let keys = Pool::new();
    let tcp = keys.key("http", "example.com", 443, None, Transport::Tcp);
    let quic = keys.key("https", "example.com", 443, None, Transport::Quic);
    assert_ne!(tcp, quic, "H2 (Tcp) and H3 (Quic) must not share a key");

    assert_eq!(
        tcp,
        keys.key("http", "example.com", 443, None, Transport::Tcp)
    );

    assert_ne!(
        tcp,
        keys.key("http", "example.com", 443, Some("p:8080"), Transport::Tcp)
    );
}

#[test]
fn kinds() {
    let tcp = Error::new(Cause::Tls).with_source(TlsError::TcpConnect(
        std::io::ErrorKind::ConnectionRefused.into(),
    ));
    let tcp = connect_err(&tcp);
    assert!(matches!(
        tcp.tls(),
        Some(TlsError::TcpConnect(err)) if err.kind() == std::io::ErrorKind::ConnectionRefused
    ));

    let handshake = Error::new(Cause::Tls).with_source(TlsError::HandshakeIo(
        std::io::ErrorKind::UnexpectedEof.into(),
    ));
    let handshake = connect_err(&handshake);
    assert!(matches!(
        handshake.tls(),
        Some(TlsError::HandshakeIo(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof
    ));

    let io = Error::new(Cause::Io)
        .with_source(std::io::Error::from(std::io::ErrorKind::ConnectionAborted));
    let io = connect_err(&io);
    assert!(
        io.io()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::ConnectionAborted)
    );
}

#[test]
fn a_joined_connect_error_keeps_its_http2_cause() {
    let goaway = Error::new(Cause::Http2).with_source(H2Error::Connection {
        code: ErrorCode::NoError,
        reason: "server shutting down".into(),
    });
    let joined = connect_err(&goaway);
    assert!(
        matches!(
            joined.h2(),
            Some(H2Error::Connection {
                code: ErrorCode::NoError,
                ..
            })
        ),
        "{joined:?}"
    );
    assert!(goaway.is_retryable() && joined.is_retryable(), "{joined:?}");
}
