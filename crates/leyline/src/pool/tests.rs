use super::*;
use crate::tls::TlsError;
use crate::{Error, Kind};

#[test]
fn transport_separates_the_keyspace() {
    let tcp = make_key("http", "example.com", 443, None, Transport::Tcp);
    let quic = make_key("https", "example.com", 443, None, Transport::Quic);
    assert_ne!(tcp, quic, "H2 (Tcp) and H3 (Quic) must not share a key");

    assert_eq!(
        tcp,
        make_key("http", "example.com", 443, None, Transport::Tcp)
    );

    assert_ne!(
        tcp,
        make_key("http", "example.com", 443, Some("p:8080"), Transport::Tcp)
    );
}

#[test]
fn kinds() {
    let tcp = Error::new(Kind::Tls).with_source(TlsError::TcpConnect(
        std::io::ErrorKind::ConnectionRefused.into(),
    ));
    let tcp = connect_err(&tcp);
    assert!(matches!(
        tcp.tls(),
        Some(TlsError::TcpConnect(err)) if err.kind() == std::io::ErrorKind::ConnectionRefused
    ));

    let handshake = Error::new(Kind::Tls).with_source(TlsError::HandshakeIo(
        std::io::ErrorKind::UnexpectedEof.into(),
    ));
    let handshake = connect_err(&handshake);
    assert!(matches!(
        handshake.tls(),
        Some(TlsError::HandshakeIo(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof
    ));

    let io = Error::new(Kind::Io)
        .with_source(std::io::Error::from(std::io::ErrorKind::ConnectionAborted));
    let io = connect_err(&io);
    assert!(
        io.io()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::ConnectionAborted)
    );
}
