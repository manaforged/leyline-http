use super::*;
use crate::Error;
use crate::tls::TlsError;

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
    let tcp = Error::Tls(TlsError::TcpConnect(
        std::io::ErrorKind::ConnectionRefused.into(),
    ));
    assert!(matches!(
        connect_err(&tcp),
        Error::Tls(TlsError::TcpConnect(err))
            if err.kind() == std::io::ErrorKind::ConnectionRefused
    ));

    let handshake = Error::Tls(TlsError::HandshakeIo(
        std::io::ErrorKind::UnexpectedEof.into(),
    ));
    assert!(matches!(
        connect_err(&handshake),
        Error::Tls(TlsError::HandshakeIo(err))
            if err.kind() == std::io::ErrorKind::UnexpectedEof
    ));

    let io = Error::Io(std::io::ErrorKind::ConnectionAborted.into());
    assert!(matches!(
        connect_err(&io),
        Error::Io(err) if err.kind() == std::io::ErrorKind::ConnectionAborted
    ));
}
