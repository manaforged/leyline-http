use super::*;
use crate::error::{Error, Kind};
use crate::h2::{ErrorCode, H2Error};
use crate::profile::Carrier;
use crate::tls::error::TlsError;

#[test]
fn transport_separates_the_keyspace() {
    let keys = Pool::new();
    let tcp = keys.key(
        crate::wire::Scheme::Http,
        "example.com",
        443,
        None,
        Carrier::Tcp,
    );
    let quic = keys.key(
        crate::wire::Scheme::Https,
        "example.com",
        443,
        None,
        Carrier::Quic,
    );
    assert_ne!(tcp, quic, "H2 (Tcp) and H3 (Quic) must not share a key");

    assert_eq!(
        tcp,
        keys.key(
            crate::wire::Scheme::Http,
            "example.com",
            443,
            None,
            Carrier::Tcp
        )
    );

    assert_ne!(
        tcp,
        keys.key(
            crate::wire::Scheme::Http,
            "example.com",
            443,
            Some(&crate::net::ProxyUrl::parse("http://p:8080").unwrap()),
            Carrier::Tcp
        )
    );
}

#[test]
fn kinds() {
    let tcp = Error::new(Kind::Tls).with_cause(TlsError::Net(crate::net::NetError::TcpConnect(
        std::io::ErrorKind::ConnectionRefused.into(),
    )));
    let tcp = tcp.duplicate();
    assert!(matches!(
        tcp.tls(),
        Some(TlsError::Net(crate::net::NetError::TcpConnect(err))) if err.kind() == std::io::ErrorKind::ConnectionRefused
    ));

    let handshake = Error::new(Kind::Tls).with_cause(TlsError::HandshakeIo(
        std::io::ErrorKind::UnexpectedEof.into(),
    ));
    let handshake = handshake.duplicate();
    assert!(matches!(
        handshake.tls(),
        Some(TlsError::HandshakeIo(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof
    ));

    let io = Error::new(Kind::Io)
        .with_source(std::io::Error::from(std::io::ErrorKind::ConnectionAborted));
    let io = io.duplicate();
    assert!(
        io.io()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::ConnectionAborted)
    );
}

#[test]
fn a_joined_connect_error_keeps_its_http2_cause() {
    let goaway = Error::new(Kind::Http2).with_cause(H2Error::Connection {
        code: ErrorCode::NoError,
        reason: "server shutting down".into(),
    });
    let joined = goaway.duplicate();
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
