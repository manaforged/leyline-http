use std::borrow::Cow;
use std::path::Path;

use leyline_bssl::pkey::PKey;
use leyline_bssl::ssl::SslContextBuilder;

use super::config::{ClientIdentity, Pem};
use super::identity::TrustKey;
use crate::tls::error::TlsError;

fn read(path: &Path) -> Result<Vec<u8>, TlsError> {
    std::fs::read(path).map_err(|err| TlsError::TrustStore(format!("{}: {err}", path.display())))
}

impl Pem {
    fn load(&self) -> Result<Cow<'_, [u8]>, TlsError> {
        match self {
            Self::File(path) => read(path).map(Cow::Owned),
            Self::Bytes(pem) => Ok(Cow::Borrowed(pem)),
        }
    }
}

pub(super) fn add_root_file(
    builder: &mut SslContextBuilder,
    path: &Path,
    loaded: &mut TrustKey,
) -> Result<usize, TlsError> {
    let pem = read(path)?;
    let added = builder
        .cert_store_mut()
        .add_pem(&pem)
        .map_err(TlsError::from_stack)?;
    loaded.root_bytes(&pem);
    Ok(added)
}

pub(super) fn set_client_identity(
    builder: &mut SslContextBuilder,
    identity: &ClientIdentity,
    loaded: &mut TrustKey,
) -> Result<(), TlsError> {
    let chain = identity.certificate_chain.load()?;
    builder
        .set_certificate_chain_pem(&chain)
        .map_err(TlsError::from_stack)?;
    let key_pem = identity.private_key.load()?;
    let key = PKey::private_key_from_pem(&key_pem).map_err(TlsError::from_stack)?;
    builder
        .set_private_key(&key)
        .map_err(TlsError::from_stack)?;
    loaded.identity_bytes(&chain);
    loaded.identity_bytes(&key_pem);
    Ok(())
}
