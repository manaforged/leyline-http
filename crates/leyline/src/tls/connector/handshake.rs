use std::time::Instant;

use leyline_bssl::ssl::SslVerifyMode;
use leyline_bssl::x509::X509VerifyError;
use tokio::net::TcpStream;

use super::FingerprintConnector;
use crate::tls::alpn::{AlpnPlan, HTTP11_WIRE};
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
        plan: AlpnPlan,
    ) -> Result<TlsStream, TlsError> {
        self.handshake_over(tcp_stream, host, session_key, plan, TlsIo::Boring)
            .await
    }

    pub(crate) async fn tls_handshake_nested(
        &self,
        inner: TlsIo,
        host: &str,
        session_key: &str,
        plan: AlpnPlan,
    ) -> Result<TlsStream, TlsError> {
        self.handshake_over(inner, host, session_key, plan, |stream| {
            TlsIo::Nested(Box::new(stream))
        })
        .await
    }

    async fn handshake_over<S>(
        &self,
        io: S,
        host: &str,
        session_key: &str,
        plan: AlpnPlan,
        wrap: impl FnOnce(leyline_bssl_tokio::SslStream<S>) -> TlsIo,
    ) -> Result<TlsStream, TlsError>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let started = Instant::now();
        let host = crate::util::bare_host(host);
        let (ssl, verification_failure) = self.prepare_ssl(host, session_key, plan)?;

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

        let ssl = stream.ssl();
        let alpn = ssl.selected_alpn_protocol().map(<[u8]>::to_vec);
        let peer_cert_der = ssl.peer_certificate().and_then(|cert| cert.to_der().ok());
        let tls_version = Some(ssl.version_str().to_string());
        let tls_cipher = ssl.current_cipher().map(|c| c.name().to_string());
        if trace::on() {
            let proto = alpn
                .as_deref()
                .map(|p| String::from_utf8_lossy(p).into_owned());
            trace::tls(
                host,
                tls_version.as_deref(),
                tls_cipher.as_deref(),
                proto.as_deref(),
                started.elapsed(),
            );
        }
        Ok(TlsStream {
            stream: wrap(stream),
            alpn,
            peer_cert_der,
            tls_version,
            tls_cipher,
        })
    }

    fn prepare_ssl(
        &self,
        host: &str,
        session_key: &str,
        plan: AlpnPlan,
    ) -> Result<(leyline_bssl::ssl::Ssl, Option<VerificationFailure>), TlsError> {
        let mut config = self
            .ssl_connector
            .configure()
            .map_err(TlsError::from_stack)?;
        if plan == AlpnPlan::Http11 {
            config
                .set_alpn_protos(HTTP11_WIRE)
                .map_err(TlsError::from_stack)?;
        }
        let mut ssl = config.into_ssl(host).map_err(TlsError::from_stack)?;
        let insecure = self.trust.accepts_invalid_certs();
        if insecure {
            ssl.set_verify(SslVerifyMode::NONE);
        }
        let pins = self.trust.pinned_leaf_sha256();
        let system_roots = self.trust.uses_system_roots();
        let verification_failure = (!insecure
            && (!pins.is_empty() || cfg!(target_os = "macos") && system_roots))
            .then(|| install_verifier(&mut ssl, pins, host, system_roots));
        if !insecure {
            self.session_cache.attach(&mut ssl, session_key)?;
        }
        self.hello.apply(&mut ssl, plan.alps())?;
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
