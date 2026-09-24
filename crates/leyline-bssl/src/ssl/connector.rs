use std::io::{Read, Write};
use std::ops::{Deref, DerefMut};

use crate::dh::Dh;
use crate::error::ErrorStack;
use crate::ssl::{
    HandshakeError, Ssl, SslContext, SslContextBuilder, SslContextRef, SslMethod, SslMode,
    SslOptions, SslRef, SslStream, SslVerifyMode,
};
use crate::version;
use std::net::IpAddr;

use super::MidHandshakeSslStream;

const FFDHE_2048: &str = "
-----BEGIN DH PARAMETERS-----
MIIBCAKCAQEA
+8yTnc4kmz75fS/jY2MMddj2gbICrsRhetPfHtXV/WVhJDP1H18GbtCFY2VVPe0a
87VXE15/V8k1mE8McODmi3fipona8+/och3xWKE2rec1MKzKT0g6eXq8CrGCsyT7
YdEIqUuyyOP7uWrat2DX9GgdT0Kj3jlN9K5W7edjcrsZCwenyO4KbXCeAvzhzffi
7MA0BM0oNC9hkXL+nOmFg/+OTxIy7vKBg8P+OxtMb61zO7X8vC7CIAXFjvGDfRaD
ssbzSibBsu/6iGtCOGEoXJf
-----END DH PARAMETERS-----
";

#[allow(clippy::inconsistent_digit_grouping)]
fn ctx(method: SslMethod) -> Result<SslContextBuilder, ErrorStack> {
    let mut ctx = SslContextBuilder::new(method)?;

    let mut opts = SslOptions::ALL
        | SslOptions::NO_COMPRESSION
        | SslOptions::NO_SSLV2
        | SslOptions::NO_SSLV3
        | SslOptions::SINGLE_DH_USE
        | SslOptions::SINGLE_ECDH_USE;
    opts &= !SslOptions::DONT_INSERT_EMPTY_FRAGMENTS;

    ctx.set_options(opts);

    let mut mode =
        SslMode::AUTO_RETRY | SslMode::ACCEPT_MOVING_WRITE_BUFFER | SslMode::ENABLE_PARTIAL_WRITE;

    if version::number() >= 0x1000_1080 {
        mode |= SslMode::RELEASE_BUFFERS;
    }

    ctx.set_mode(mode);

    Ok(ctx)
}

#[derive(Clone, Debug)]
pub struct SslConnector(SslContext);

impl SslConnector {
    pub fn builder(method: SslMethod) -> Result<SslConnectorBuilder, ErrorStack> {
        let mut ctx = ctx(method)?;
        ctx.set_default_verify_paths()?;
        ctx.set_cipher_list(
            "DEFAULT:!aNULL:!eNULL:!MD5:!3DES:!DES:!RC4:!IDEA:!SEED:!aDSS:!SRP:!PSK",
        )?;
        setup_verify(&mut ctx);

        Ok(SslConnectorBuilder(ctx))
    }

    pub fn bare_builder(method: SslMethod) -> Result<SslConnectorBuilder, ErrorStack> {
        let mut ctx = ctx(method)?;
        ctx.set_cipher_list(
            "DEFAULT:!aNULL:!eNULL:!MD5:!3DES:!DES:!RC4:!IDEA:!SEED:!aDSS:!SRP:!PSK",
        )?;

        Ok(SslConnectorBuilder(ctx))
    }

    pub fn setup_connect<S>(
        &self,
        domain: &str,
        stream: S,
    ) -> Result<MidHandshakeSslStream<S>, ErrorStack>
    where
        S: Read + Write,
    {
        self.configure()?.setup_connect(domain, stream)
    }

    pub fn connect<S>(&self, domain: &str, stream: S) -> Result<SslStream<S>, HandshakeError<S>>
    where
        S: Read + Write,
    {
        self.setup_connect(domain, stream)
            .map_err(HandshakeError::SetupFailure)?
            .handshake()
    }

    pub fn configure(&self) -> Result<ConnectConfiguration, ErrorStack> {
        Ssl::new(&self.0).map(|ssl| ConnectConfiguration {
            ssl,
            sni: true,
            verify_hostname: true,
        })
    }

    #[must_use]
    pub fn into_context(self) -> SslContext {
        self.0
    }

    #[must_use]
    pub fn context(&self) -> &SslContextRef {
        &self.0
    }
}

pub struct SslConnectorBuilder(SslContextBuilder);

impl SslConnectorBuilder {
    #[must_use]
    pub fn build(self) -> SslConnector {
        SslConnector(self.0.build())
    }
}

impl Deref for SslConnectorBuilder {
    type Target = SslContextBuilder;

    fn deref(&self) -> &SslContextBuilder {
        &self.0
    }
}

impl DerefMut for SslConnectorBuilder {
    fn deref_mut(&mut self) -> &mut SslContextBuilder {
        &mut self.0
    }
}

pub struct ConnectConfiguration {
    ssl: Ssl,
    sni: bool,
    verify_hostname: bool,
}

impl ConnectConfiguration {
    #[must_use]
    pub fn use_server_name_indication(mut self, use_sni: bool) -> ConnectConfiguration {
        self.set_use_server_name_indication(use_sni);
        self
    }

    pub fn set_use_server_name_indication(&mut self, use_sni: bool) {
        self.sni = use_sni;
    }

    #[must_use]
    pub fn verify_hostname(mut self, verify_hostname: bool) -> ConnectConfiguration {
        self.set_verify_hostname(verify_hostname);
        self
    }

    pub fn set_verify_hostname(&mut self, verify_hostname: bool) {
        self.verify_hostname = verify_hostname;
    }

    pub fn into_ssl(mut self, domain: &str) -> Result<Ssl, ErrorStack> {
        if self.sni && domain.parse::<IpAddr>().is_err() {
            self.ssl.set_hostname(domain)?;
        }

        if self.verify_hostname {
            setup_verify_hostname(&mut self.ssl, domain)?;
        }

        Ok(self.ssl)
    }

    pub fn setup_connect<S>(
        self,
        domain: &str,
        stream: S,
    ) -> Result<MidHandshakeSslStream<S>, ErrorStack>
    where
        S: Read + Write,
    {
        Ok(self.into_ssl(domain)?.setup_connect(stream))
    }

    pub fn connect<S>(self, domain: &str, stream: S) -> Result<SslStream<S>, HandshakeError<S>>
    where
        S: Read + Write,
    {
        self.setup_connect(domain, stream)
            .map_err(HandshakeError::SetupFailure)?
            .handshake()
    }
}

impl Deref for ConnectConfiguration {
    type Target = SslRef;

    fn deref(&self) -> &SslRef {
        &self.ssl
    }
}

impl DerefMut for ConnectConfiguration {
    fn deref_mut(&mut self) -> &mut SslRef {
        &mut self.ssl
    }
}

#[derive(Clone)]
pub struct SslAcceptor(SslContext);

impl SslAcceptor {
    pub fn mozilla_intermediate_v5(method: SslMethod) -> Result<SslAcceptorBuilder, ErrorStack> {
        let mut ctx = ctx(method)?;
        ctx.set_options(SslOptions::NO_TLSV1 | SslOptions::NO_TLSV1_1);
        let dh = Dh::params_from_pem(FFDHE_2048.as_bytes())?;
        ctx.set_tmp_dh(&dh)?;
        ctx.set_cipher_list(
            "ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES256-GCM-SHA384:\
             ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-CHACHA20-POLY1305:ECDHE-RSA-CHACHA20-POLY1305:\
             DHE-RSA-AES128-GCM-SHA256:DHE-RSA-AES256-GCM-SHA384"
        )?;
        Ok(SslAcceptorBuilder(ctx))
    }

    pub fn mozilla_intermediate(method: SslMethod) -> Result<SslAcceptorBuilder, ErrorStack> {
        let mut ctx = ctx(method)?;
        ctx.set_options(SslOptions::CIPHER_SERVER_PREFERENCE);
        ctx.set_options(SslOptions::NO_TLSV1_3);
        let dh = Dh::params_from_pem(FFDHE_2048.as_bytes())?;
        ctx.set_tmp_dh(&dh)?;
        ctx.set_cipher_list(
            "ECDHE-ECDSA-CHACHA20-POLY1305:ECDHE-RSA-CHACHA20-POLY1305:ECDHE-ECDSA-AES128-GCM-SHA256:\
             ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384:\
             DHE-RSA-AES128-GCM-SHA256:DHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-AES128-SHA256:ECDHE-RSA-AES128-SHA256:\
             ECDHE-ECDSA-AES128-SHA:ECDHE-RSA-AES256-SHA384:ECDHE-RSA-AES128-SHA:ECDHE-ECDSA-AES256-SHA384:\
             ECDHE-ECDSA-AES256-SHA:ECDHE-RSA-AES256-SHA:DHE-RSA-AES128-SHA256:DHE-RSA-AES128-SHA:\
             DHE-RSA-AES256-SHA256:DHE-RSA-AES256-SHA:ECDHE-ECDSA-DES-CBC3-SHA:ECDHE-RSA-DES-CBC3-SHA:\
             EDH-RSA-DES-CBC3-SHA:AES128-GCM-SHA256:AES256-GCM-SHA384:AES128-SHA256:AES256-SHA256:AES128-SHA:\
             AES256-SHA:DES-CBC3-SHA:!DSS",
        )?;
        Ok(SslAcceptorBuilder(ctx))
    }

    pub fn mozilla_modern(method: SslMethod) -> Result<SslAcceptorBuilder, ErrorStack> {
        let mut ctx = ctx(method)?;
        ctx.set_options(
            SslOptions::CIPHER_SERVER_PREFERENCE | SslOptions::NO_TLSV1 | SslOptions::NO_TLSV1_1,
        );
        ctx.set_options(SslOptions::NO_TLSV1_3);
        ctx.set_cipher_list(
            "ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-CHACHA20-POLY1305:\
             ECDHE-RSA-CHACHA20-POLY1305:ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:\
             ECDHE-ECDSA-AES256-SHA384:ECDHE-RSA-AES256-SHA384:ECDHE-ECDSA-AES128-SHA256:ECDHE-RSA-AES128-SHA256",
        )?;
        Ok(SslAcceptorBuilder(ctx))
    }

    pub fn setup_accept<S>(&self, stream: S) -> Result<MidHandshakeSslStream<S>, ErrorStack>
    where
        S: Read + Write,
    {
        let ssl = Ssl::new(&self.0)?;

        Ok(ssl.setup_accept(stream))
    }

    pub fn accept<S>(&self, stream: S) -> Result<SslStream<S>, HandshakeError<S>>
    where
        S: Read + Write,
    {
        self.setup_accept(stream)
            .map_err(HandshakeError::SetupFailure)?
            .handshake()
    }

    #[must_use]
    pub fn into_context(self) -> SslContext {
        self.0
    }

    #[must_use]
    pub fn context(&self) -> &SslContextRef {
        &self.0
    }
}

pub struct SslAcceptorBuilder(SslContextBuilder);

impl SslAcceptorBuilder {
    #[must_use]
    pub fn build(self) -> SslAcceptor {
        SslAcceptor(self.0.build())
    }
}

impl Deref for SslAcceptorBuilder {
    type Target = SslContextBuilder;

    fn deref(&self) -> &SslContextBuilder {
        &self.0
    }
}

impl DerefMut for SslAcceptorBuilder {
    fn deref_mut(&mut self) -> &mut SslContextBuilder {
        &mut self.0
    }
}

fn setup_verify(ctx: &mut SslContextBuilder) {
    ctx.set_verify(SslVerifyMode::PEER);
}

fn setup_verify_hostname(ssl: &mut SslRef, domain: &str) -> Result<(), ErrorStack> {
    use crate::x509::verify::X509CheckFlags;

    let param = ssl.param_mut();
    param.set_hostflags(X509CheckFlags::NO_PARTIAL_WILDCARDS);
    match domain.parse() {
        Ok(ip) => param.set_ip(ip),
        Err(_) => param.set_host(domain),
    }
}
