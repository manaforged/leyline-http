use super::*;

// The keystone of H2/H3 coexistence: the same destination under different
// transports must be DISTINCT pool keys, or an H3 install clobbers a live
// H2 entry (and vice versa). If this ever asserts equal, the pool collapsed
// back to one-entry-per-host and the collision is back.
#[test]
fn transport_separates_the_keyspace() {
    let tcp = make_key("http", "example.com", 443, None, Transport::Tcp);
    let quic = make_key("https", "example.com", 443, None, Transport::Quic);
    assert_ne!(tcp, quic, "H2 (Tcp) and H3 (Quic) must not share a key");

    // Same transport + destination → same key (so reuse still works).
    assert_eq!(
        tcp,
        make_key("http", "example.com", 443, None, Transport::Tcp)
    );

    // The proxy leg still participates in identity.
    assert_ne!(
        tcp,
        make_key("http", "example.com", 443, Some("p:8080"), Transport::Tcp)
    );
}

#[test]
fn owned_connect_error_preserves_transport_variant_and_io_kind() {
    let tcp = crate::Error::Tls(crate::tls::TlsError::TcpConnect(
        std::io::ErrorKind::ConnectionRefused.into(),
    ));
    assert!(matches!(
        owned_connect_err(&tcp),
        crate::Error::Tls(crate::tls::TlsError::TcpConnect(err))
            if err.kind() == std::io::ErrorKind::ConnectionRefused
    ));

    let handshake = crate::Error::Tls(crate::tls::TlsError::HandshakeIo(
        std::io::ErrorKind::UnexpectedEof.into(),
    ));
    assert!(matches!(
        owned_connect_err(&handshake),
        crate::Error::Tls(crate::tls::TlsError::HandshakeIo(err))
            if err.kind() == std::io::ErrorKind::UnexpectedEof
    ));

    let io = crate::Error::Io(std::io::ErrorKind::ConnectionAborted.into());
    assert!(matches!(
        owned_connect_err(&io),
        crate::Error::Io(err) if err.kind() == std::io::ErrorKind::ConnectionAborted
    ));
}
