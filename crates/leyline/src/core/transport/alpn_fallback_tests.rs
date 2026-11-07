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
    let closed = crate::pool::h1::closed("before headers");
    assert!(
        closed.cause() == Kind::Io
            && closed
                .io()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::UnexpectedEof),
        "a closed connection must be Io(UnexpectedEof), got {closed:?}"
    );
    assert!(closed.is_connection_closed());
    let framing = crate::pool::h1::malformed("invalid Transfer-Encoding: connection-close");
    assert!(
        framing.cause() == Kind::Io
            && framing
                .io()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::InvalidData),
        "a malformed response must be Io(InvalidData), got {framing:?}"
    );
    assert!(!framing.is_connection_closed());

    let pinning = Error::from(crate::tls::TlsError::Pinning("mismatch".into()));
    assert!(
        matches!(pinning.tls(), Some(crate::tls::TlsError::Pinning(_))),
        "H1 must preserve typed TLS failures, got {pinning:?}"
    );
    assert!(!pinning.is_connection_closed());
}
