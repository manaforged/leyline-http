use super::Error;
use crate::tls::TlsError;

#[test]
fn timeout_variant_is_timeout_not_connect() {
    assert!(Error::Timeout.is_timeout());
    assert!(!Error::Timeout.is_connect());
}

#[test]
fn timed_out_io_is_timeout_not_connect() {
    let err = Error::Io(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "read timed out",
    ));
    assert!(err.is_timeout());
    assert!(!err.is_connect());
}

#[test]
fn body_io_is_not_connect() {
    let err = Error::Io(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        "body eof",
    ));
    assert!(!err.is_connect());
    assert!(!err.is_timeout());
}

#[test]
fn file_io_is_not_connect() {
    let err = Error::Io(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no such file",
    ));
    assert!(!err.is_connect());
}

#[test]
fn refused_io_is_connect() {
    let err = Error::Io(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "refused",
    ));
    assert!(err.is_connect());
    assert!(!err.is_timeout());
}

#[test]
fn tls_handshake_is_connect() {
    let err = Error::Tls(TlsError::Handshake("alert".into()));
    assert!(err.is_connect());
}

#[test]
fn cert_mismatch_is_not_connect() {
    let err = Error::Tls(TlsError::Hostname("wrong host".into()));
    assert!(!err.is_connect());
}
