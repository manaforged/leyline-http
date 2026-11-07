#[derive(Debug, Clone, Default)]
pub struct TlsInfo(std::sync::Arc<TlsFacts>);

#[derive(Debug, Default)]
#[non_exhaustive]
pub struct TlsFacts {
    pub peer_cert_der: Option<Vec<u8>>,
    pub version: Option<String>,
    pub cipher: Option<String>,
}

impl From<TlsFacts> for TlsInfo {
    fn from(facts: TlsFacts) -> Self {
        Self(std::sync::Arc::new(facts))
    }
}

impl std::ops::Deref for TlsInfo {
    type Target = TlsFacts;

    fn deref(&self) -> &TlsFacts {
        &self.0
    }
}
