use std::time::Instant;

use leyline_bssl::ssl::SslVerifyMode;
use leyline_bssl::x509::X509VerifyError;
use tokio::net::TcpStream;

use super::FingerprintConnector;
use crate::tls::error::TlsError;
use crate::tls::trust::{
    TrustFailure, VerificationFailure, install_verifier, take_verification_failure,
};
use crate::tls::{TlsIo, TlsStream};
use crate::trace;

impl FingerprintConnector {
    pub(crate) async fn tls_handshake(
        &self,
        tcp_stream: TcpStream,
        host: &str,
        session_key: &str,
        include_alps: bool,
    ) -> Result<TlsStream, TlsError> {
        let (stream, meta) = self
            .handshake_over(tcp_stream, host, session_key, include_alps)
            .await?;
        Ok(meta.into_tls_stream(TlsIo::Boring(stream)))
    }

    pub(crate) async fn tls_handshake_nested(
        &self,
        inner: TlsIo,
        host: &str,
        session_key: &str,
        include_alps: bool,
    ) -> Result<TlsStream, TlsError> {
        let (stream, meta) = self
            .handshake_over(inner, host, session_key, include_alps)
            .await?;
        Ok(meta.into_tls_stream(TlsIo::Nested(Box::new(stream))))
    }

    async fn handshake_over<S>(
        &self,
        io: S,
        host: &str,
        session_key: &str,
        include_alps: bool,
    ) -> Result<(leyline_bssl_tokio::SslStream<S>, TlsMeta), TlsError>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let started = Instant::now();
        let host = crate::util::bare_host(host);
        let (ssl, verification_failure) = self.prepare_ssl(host, session_key, include_alps)?;

        let stream = match leyline_bssl_tokio::SslStreamBuilder::new(ssl, io)
            .connect()
            .await
        {
            Ok(stream) => stream,
            Err(e) => {
                let verify_error = e.ssl().and_then(|ssl| ssl.verify_result().err());
                return Err(classify_handshake(
                    verification_failure.as_ref(),
                    verify_error,
                    e,
                ));
            }
        };

        let meta = TlsMeta::of(stream.ssl());
        if trace::on() {
            meta.trace(host, started);
        }
        Ok((stream, meta))
    }

    fn prepare_ssl(
        &self,
        host: &str,
        session_key: &str,
        include_alps: bool,
    ) -> Result<(leyline_bssl::ssl::Ssl, Option<VerificationFailure>), TlsError> {
        let mut config = self
            .ssl_connector
            .configure()
            .map_err(TlsError::from_stack)?;
        if !include_alps {
            config
                .set_alpn_protos(crate::tls::alpn::HTTP11_WIRE)
                .map_err(TlsError::from_stack)?;
        }
        let mut ssl = config.into_ssl(host).map_err(TlsError::from_stack)?;
        let insecure = self.insecure_mode();
        if insecure {
            ssl.set_verify(SslVerifyMode::NONE);
        }
        let verification_failure = (!insecure
            && (!self.pins.is_empty() || cfg!(target_os = "macos") && self.system_roots))
            .then(|| install_verifier(&mut ssl, &self.pins, host, self.system_roots));
        if !insecure {
            self.session_cache.attach(&mut ssl, session_key)?;
        }
        self.hello.apply(&mut ssl, include_alps)?;
        Ok((ssl, verification_failure))
    }
}

fn classify_handshake<S>(
    failure: Option<&VerificationFailure>,
    verify_error: Option<X509VerifyError>,
    error: leyline_bssl_tokio::HandshakeError<S>,
) -> TlsError {
    let verify_error = verify_error.filter(|error| *error != X509VerifyError::INVALID_CALL);
    match failure.and_then(take_verification_failure) {
        Some(TrustFailure::Certificate) => certificate(verify_error, &error),
        Some(TrustFailure::Hostname) => TlsError::Hostname(error.to_string()),
        Some(TrustFailure::Pinning) => TlsError::Pinning(error.to_string()),
        None if matches!(
            verify_error,
            Some(X509VerifyError::HOSTNAME_MISMATCH | X509VerifyError::IP_ADDRESS_MISMATCH)
        ) =>
        {
            TlsError::Hostname(error.to_string())
        }
        None if verify_error.is_some() => certificate(verify_error, &error),
        None => TlsError::from_handshake(&error),
    }
}

fn certificate<S>(
    verify_error: Option<X509VerifyError>,
    error: &leyline_bssl_tokio::HandshakeError<S>,
) -> TlsError {
    TlsError::Certificate {
        verify_code: verify_error.map(|e| e.as_raw()),
        reason: verify_error.map(|e| e.error_string()),
        detail: error.to_string(),
    }
}

struct TlsMeta {
    alpn: Option<Vec<u8>>,
    peer_cert_der: Option<Vec<u8>>,
    tls_version: Option<String>,
    tls_cipher: Option<String>,
}

impl TlsMeta {
    fn of(ssl: &leyline_bssl::ssl::SslRef) -> Self {
        Self {
            alpn: ssl.selected_alpn_protocol().map(<[u8]>::to_vec),
            peer_cert_der: ssl.peer_certificate().and_then(|cert| cert.to_der().ok()),
            tls_version: Some(ssl.version_str().to_string()),
            tls_cipher: ssl.current_cipher().map(|c| c.name().to_string()),
        }
    }

    fn trace(&self, host: &str, started: Instant) {
        let proto = self
            .alpn
            .as_deref()
            .map(|p| String::from_utf8_lossy(p).into_owned());
        trace::tls(
            host,
            self.tls_version.as_deref(),
            self.tls_cipher.as_deref(),
            proto.as_deref(),
            started.elapsed(),
        );
    }

    fn into_tls_stream(self, stream: TlsIo) -> TlsStream {
        TlsStream {
            stream,
            alpn: self.alpn,
            peer_cert_der: self.peer_cert_der,
            tls_version: self.tls_version,
            tls_cipher: self.tls_cipher,
        }
    }
}
