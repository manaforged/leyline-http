use super::*;

#[test]
fn alpn_mismatch_detected() {
    assert!(is_h2_alpn_mismatch(&Error::AlpnMismatch {
        negotiated: "none".into()
    }));
    assert!(is_h2_alpn_mismatch(&Error::AlpnMismatch {
        negotiated: "http/1.1".into()
    }));
    assert!(!is_h2_alpn_mismatch(&Error::Http("404 not found".into())));
    assert!(!is_h2_alpn_mismatch(&Error::Http2(
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
        matches!(&closed, Error::Io(e) if e.kind() == std::io::ErrorKind::UnexpectedEof),
        "ConnectionClosed must map to Io(UnexpectedEof), got {closed:?}"
    );
    assert!(closed.is_connection_closed());
    let framing = h1_error_to_core(H1PooledError::Http(
        "invalid Transfer-Encoding: connection-close".into(),
    ));
    assert!(matches!(framing, Error::Http(_)));
    assert!(!framing.is_connection_closed());

    let pinning = h1_error_to_core(H1PooledError::Tls(crate::tls::TlsError::Pinning(
        "mismatch".into(),
    )));
    assert!(
        matches!(&pinning, Error::Tls(crate::tls::TlsError::Pinning(_))),
        "H1 must preserve typed TLS failures, got {pinning:?}"
    );
    assert!(!pinning.is_connection_closed());
}
