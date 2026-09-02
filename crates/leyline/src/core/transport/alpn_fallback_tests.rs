use super::*;

#[test]
fn alpn_mismatch_detected() {
    assert!(is_h2_alpn_mismatch(
        &Error::new(Kind::Http2).with_alpn("none")
    ));
    assert!(is_h2_alpn_mismatch(
        &Error::new(Kind::Http2).with_alpn("http/1.1")
    ));
    assert!(!is_h2_alpn_mismatch(
        &Error::new(Kind::Request).with_message("404 not found")
    ));
    assert!(!is_h2_alpn_mismatch(&Error::new(Kind::Http2).with_source(
        crate::h2::H2Error::Stream {
            stream_id: 1,
            code: crate::h2::error::ErrorCode::RefusedStream,
        }
    )));
}

#[test]
fn h1_connection_closed_is_typed_retryable_but_framing_is_not() {
    use crate::pool::H1PooledError;
    let closed = h1_error_to_core(H1PooledError::ConnectionClosed("before headers".into()));
    assert!(
        closed.kind() == Kind::Io
            && closed
                .io()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::UnexpectedEof),
        "ConnectionClosed must map to Io(UnexpectedEof), got {closed:?}"
    );
    assert!(closed.is_connection_closed());
    let framing = h1_error_to_core(H1PooledError::Http(
        "invalid Transfer-Encoding: connection-close".into(),
    ));
    assert_eq!(framing.kind(), Kind::Request);
    assert!(framing.message().is_some());
    assert!(!framing.is_connection_closed());

    let pinning = h1_error_to_core(H1PooledError::Tls(crate::tls::TlsError::Pinning(
        "mismatch".into(),
    )));
    assert!(
        matches!(pinning.tls(), Some(crate::tls::TlsError::Pinning(_))),
        "H1 must preserve typed TLS failures, got {pinning:?}"
    );
    assert!(!pinning.is_connection_closed());
}
