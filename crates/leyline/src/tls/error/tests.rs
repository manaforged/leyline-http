use super::TlsError;
use std::io::{Error, ErrorKind};

#[test]
fn connect_phase_is_retryable() {
    assert!(TlsError::TcpConnect(Error::from(ErrorKind::ConnectionRefused)).is_retryable());
    assert!(TlsError::Dns(Error::from(ErrorKind::NotFound)).is_retryable());
    assert!(TlsError::HandshakeIo(Error::from(ErrorKind::UnexpectedEof)).is_retryable());
    assert!(TlsError::Handshake("alert handshake failure".into()).is_retryable());
    assert!(TlsError::SslConnect("ssl stream init".into()).is_retryable());
}

#[test]
fn identity_is_permanent() {
    assert!(!TlsError::Certificate("untrusted".into()).is_retryable());
    assert!(!TlsError::Hostname("mismatch".into()).is_retryable());
    assert!(!TlsError::Pinning("pin".into()).is_retryable());
    assert!(!TlsError::SslConfig("builder".into()).is_retryable());
    assert!(!TlsError::Profile("bad toml".into()).is_retryable());
    assert!(!TlsError::TrustStore("empty".into()).is_retryable());
}
