#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TlsError {
    #[error("ssl config: {0}")]
    SslConfig(String),

    #[error("ssl handshake: {0}")]
    Handshake(String),

    #[error("ssl handshake io: {0}")]
    HandshakeIo(#[source] std::io::Error),

    #[error("certificate: {0}")]
    Certificate(String),

    #[error("hostname: {0}")]
    Hostname(String),

    #[error("certificate pin: {0}")]
    Pinning(String),

    #[error("tcp connect: {0}")]
    TcpConnect(#[source] std::io::Error),

    #[error("dns: {0}")]
    Dns(#[source] std::io::Error),

    #[error("ssl connect: {0}")]
    SslConnect(String),

    #[error("profile: {0}")]
    Profile(String),

    #[error("trust store: {0}")]
    TrustStore(String),
}

fn transport_eof(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("unexpected eof")
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
        || lower.contains("connection aborted")
}

impl TlsError {
    pub(crate) fn from_handshake<S>(e: &leyline_bssl_tokio::HandshakeError<S>) -> Self {
        if let Some(io) = e.as_io_error() {
            return Self::HandshakeIo(std::io::Error::new(io.kind(), io.to_string()));
        }
        let msg = e.to_string();
        if transport_eof(&msg) {
            Self::HandshakeIo(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, msg))
        } else {
            Self::Handshake(msg)
        }
    }

    pub(crate) fn from_stack(e: leyline_bssl::error::ErrorStack) -> Self {
        Self::SslConfig(e.to_string())
    }
}
