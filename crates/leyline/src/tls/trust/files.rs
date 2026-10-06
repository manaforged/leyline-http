use std::path::Path;

use leyline_bssl::pkey::PKey;
use leyline_bssl::ssl::SslContextBuilder;

use super::config::ClientIdentity;
use super::identity::LoadedTrust;
use crate::tls::error::TlsError;

fn read(path: &Path) -> Result<Vec<u8>, TlsError> {
    std::fs::read(path).map_err(|err| TlsError::TrustStore(format!("{}: {err}", path.display())))
}

pub(super) fn add_root_file(
    builder: &mut SslContextBuilder,
    path: &Path,
    loaded: &mut LoadedTrust,
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
    loaded: &mut LoadedTrust,
) -> Result<(), TlsError> {
    let chain = read(&identity.certificate_chain_file)?;
    builder
        .set_certificate_chain_pem(&chain)
        .map_err(TlsError::from_stack)?;
    let key_pem = read(&identity.private_key_file)?;
    let key = PKey::private_key_from_pem(&key_pem).map_err(TlsError::from_stack)?;
    builder
        .set_private_key(&key)
        .map_err(TlsError::from_stack)?;
    loaded.identity_bytes(&chain);
    loaded.identity_bytes(&key_pem);
    Ok(())
}
