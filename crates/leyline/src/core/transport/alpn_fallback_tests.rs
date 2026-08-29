use super::*;

#[test]
fn alpn_mismatch_detected() {
    // The pool surfaces the ALPN decline as the typed Error::AlpnMismatch,
    // which must trip the HTTP/1.1 fallback regardless of which protocol
    // (or none) the peer negotiated. Without it, proxyless navigations to
    // hosts that decline h2 ALPN on a cookieless interstitial hard-error
    // instead of falling back.
    assert!(is_h2_alpn_mismatch(&Error::AlpnMismatch {
        negotiated: "none".into()
    }));
    assert!(is_h2_alpn_mismatch(&Error::AlpnMismatch {
        negotiated: "http/1.1".into()
    }));
    // Unrelated errors must NOT trigger a fallback — neither a generic
    // HTTP error nor a non-ALPN HTTP/2 transport error.
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
    // Mid-exchange EOF → typed Io(UnexpectedEof), classified as a
    // connection-closed (so the retry engine's typed Io arm retries it).
    let closed = h1_error_to_core(H1PooledError::ConnectionClosed("before headers".into()));
    assert!(
        matches!(&closed, Error::Io(e) if e.kind() == std::io::ErrorKind::UnexpectedEof),
        "ConnectionClosed must map to Io(UnexpectedEof), got {closed:?}"
    );
    assert!(closed.is_connection_closed());
    // A framing error whose message interpolates attacker-controlled header
    // bytes containing "connection" must NOT be classified as
    // connection-closed — a framing error must never be classified as
    // a retryable connection failure.
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
