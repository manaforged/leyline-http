//! TLS connector that creates fingerprinted connections from browser profiles.

use std::net::ToSocketAddrs;
use std::pin::Pin;

use boring2::ssl::{
    CertificateCompressionAlgorithm, CertificateCompressor, SslConnector, SslMethod, SslVerifyMode,
};
use tokio::net::TcpStream;

/// Brotli cert decompression (advertises compress_certificate extension in ClientHello).
struct BrotliDecompressor;

impl CertificateCompressor for BrotliDecompressor {
    const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::BROTLI;
    const CAN_COMPRESS: bool = false;
    const CAN_DECOMPRESS: bool = true;

    fn decompress<W: std::io::Write>(&self, input: &[u8], output: &mut W) -> std::io::Result<()> {
        let mut decoder = brotli::Decompressor::new(input, 4096);
        std::io::copy(&mut decoder, output)?;
        Ok(())
    }
}

use leyline_profile::BrowserProfile;
use leyline_tcp::TcpProfile;

use crate::error::TlsError;
use crate::TlsStream;

/// Creates TLS connections matching a browser's fingerprint.
pub struct FingerprintConnector {
    ssl_connector: SslConnector,
    tcp_profile: TcpProfile,
    ech_grease: bool,
}

impl FingerprintConnector {
    /// Build a connector from a browser profile and TCP profile.
    pub fn new(profile: &BrowserProfile, tcp: TcpProfile) -> Result<Self, TlsError> {
        let mut builder = SslConnector::builder(SslMethod::tls_client())?;

        let tls = &profile.tls;

        // Cipher suites.
        let cipher_str = tls.ciphers.join(":");
        builder.set_cipher_list(&cipher_str)?;

        // Curves.
        let curves_str = tls
            .curves
            .iter()
            .map(|c| boring_curve_name(c))
            .collect::<Vec<_>>()
            .join(":");
        builder.set_curves_list(&curves_str)?;

        // Signature algorithms.
        let sigalgs_str = tls.sigalgs.join(":");
        builder.set_sigalgs_list(&sigalgs_str)?;

        // OCSP stapling.
        if tls.ocsp_stapling {
            builder.enable_ocsp_stapling();
        }

        // Signed certificate timestamps.
        if tls.signed_cert_timestamps {
            builder.enable_signed_cert_timestamps();
        }

        // Certificate compression (advertises compress_certificate extension).
        for algo in &tls.cert_compression {
            if algo == "brotli" {
                builder.add_certificate_compression_algorithm(BrotliDecompressor)?;
            }
        }

        // Extension permutation (random shuffle vs fixed order).
        if tls.permute_extensions {
            builder.set_permute_extensions(true);
        }
        // Note: set_extension_permutation() takes &[ExtensionType], not &[u8].
        // We'd need to map u8 indices to ExtensionType. For now, permute_extensions
        // covers Chrome (random) and we'll add fixed ordering for Firefox/Safari later.

        // GREASE.
        let needs_grease = tls.ech_grease || tls.permute_extensions;
        builder.set_grease_enabled(needs_grease);

        // ALPN — advertise h2 and http/1.1.
        builder.set_alpn_protos(b"\x02h2\x08http/1.1")?;

        // Verification.
        builder.set_verify(SslVerifyMode::PEER);

        Ok(Self {
            ssl_connector: builder.build(),
            tcp_profile: tcp,
            ech_grease: tls.ech_grease,
        })
    }

    /// Connect to a host:port, applying TCP fingerprint and TLS handshake.
    pub async fn connect(&self, host: &str, port: u16) -> Result<TlsStream, TlsError> {
        // Resolve DNS.
        let addr_str = format!("{}:{}", host, port);
        let sock_addr = tokio::task::spawn_blocking(move || {
            addr_str
                .to_socket_addrs()
                .map_err(TlsError::Dns)?
                .next()
                .ok_or_else(|| {
                    TlsError::Dns(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "no addresses resolved",
                    ))
                })
        })
        .await
        .map_err(|e| TlsError::Dns(std::io::Error::other(e)))??;

        // Create socket via socket2 for TCP fingerprinting.
        let domain = match sock_addr {
            std::net::SocketAddr::V4(_) => socket2::Domain::IPV4,
            std::net::SocketAddr::V6(_) => socket2::Domain::IPV6,
        };
        let socket = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))
            .map_err(TlsError::TcpConnect)?;

        // Apply TCP fingerprint before connect.
        self.tcp_profile.apply(&socket);
        socket.set_nonblocking(true).map_err(TlsError::TcpConnect)?;

        // TCP connect.
        match socket.connect(&sock_addr.into()) {
            Ok(()) => {}
            Err(e) if e.raw_os_error() == Some(libc::EINPROGRESS) => {}
            Err(e) => return Err(TlsError::TcpConnect(e)),
        }
        let std_stream: std::net::TcpStream = socket.into();
        let tcp_stream = TcpStream::from_std(std_stream).map_err(TlsError::TcpConnect)?;

        // Wait for TCP connect to complete.
        tcp_stream.writable().await.map_err(TlsError::TcpConnect)?;
        if let Some(e) = tcp_stream.take_error().map_err(TlsError::TcpConnect)? {
            return Err(TlsError::TcpConnect(e));
        }

        // Configure per-connection SSL.
        let mut ssl = self
            .ssl_connector
            .configure()
            .map_err(TlsError::Handshake)?
            .into_ssl(host)
            .map_err(TlsError::Handshake)?;

        // Per-connection settings.
        if self.ech_grease {
            ssl.set_enable_ech_grease(true);
        }

        // TLS handshake.
        let mut stream =
            tokio_boring2::SslStream::new(ssl, tcp_stream).map_err(TlsError::Handshake)?;

        Pin::new(&mut stream)
            .connect()
            .await
            .map_err(|e| TlsError::SslConnect(e.to_string()))?;

        // Extract ALPN.
        let alpn = stream.ssl().selected_alpn_protocol().map(|p| p.to_vec());

        Ok(TlsStream { stream, alpn })
    }
}

/// Map profile curve names to BoringSSL curve names.
fn boring_curve_name(name: &str) -> &str {
    match name {
        "X25519_MLKEM768" => "X25519MLKEM768",
        "X25519" => "X25519",
        "SECP256R1" => "P-256",
        "SECP384R1" => "P-384",
        "SECP521R1" => "P-521",
        other => other,
    }
}

impl std::fmt::Debug for FingerprintConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FingerprintConnector")
            .field("tcp_profile", &self.tcp_profile)
            .field("ech_grease", &self.ech_grease)
            .finish_non_exhaustive()
    }
}
