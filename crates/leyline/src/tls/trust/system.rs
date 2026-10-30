use leyline_bssl::ssl::SslContextBuilder;
#[cfg(windows)]
use leyline_bssl::x509::X509;

use crate::tls::error::TlsError;

fn wire_system_trust(builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    #[cfg(windows)]
    {
        wire_windows_system_trust(builder)
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        wire_linux_system_trust(builder)
    }
}

#[cfg(windows)]
pub(super) fn wire_system_trust_cached(
    builder: &mut SslContextBuilder,
    config: &super::Tls,
    env_roots_loaded: bool,
) -> Result<(), TlsError> {
    if !env_roots_loaded && config.ca_files.is_empty() && config.ca_der.is_empty() {
        builder.set_cert_store_ref(cached_windows_system_store()?);
        return Ok(());
    }
    wire_system_trust(builder)
}

#[cfg(all(not(windows), not(target_os = "macos")))]
pub(super) fn wire_system_trust_cached(
    builder: &mut SslContextBuilder,
    _config: &super::Tls,
    _env_roots_loaded: bool,
) -> Result<(), TlsError> {
    wire_system_trust(builder)
}

#[cfg(windows)]
fn cached_windows_system_store() -> Result<&'static leyline_bssl::x509::store::X509Store, TlsError>
{
    use leyline_bssl::x509::store::{X509Store, X509StoreBuilder};
    use std::sync::OnceLock;

    static STORE: OnceLock<X509Store> = OnceLock::new();
    if let Some(s) = STORE.get() {
        return Ok(s);
    }

    let roots = windows_roots()?;
    let mut store = X509StoreBuilder::new()
        .map_err(|e| TlsError::TrustStore(format!("X509 store allocation failed: {e}")))?;
    let (loaded, _) = bridge_windows_roots(&mut store, &roots, |_, _| {});
    if loaded == 0 {
        return Err(TlsError::TrustStore(
            "Windows system ROOT store bridged zero certificates".into(),
        ));
    }
    tracing::info!(
        target: "leyline::tls::trust",
        loaded,
        "Windows system ROOT store parsed and cached for reuse"
    );
    Ok(STORE.get_or_init(|| store.build()))
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn wire_linux_system_trust(builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    const BUNDLES: &[&str] = &[
        "/etc/ssl/certs/ca-certificates.crt",
        "/etc/pki/tls/certs/ca-bundle.crt",
        "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
        "/etc/ssl/ca-bundle.pem",
        "/etc/ssl/cert.pem",
    ];
    for &path in BUNDLES {
        if std::path::Path::new(path).exists() {
            return builder
                .set_ca_file(path)
                .map_err(|e| TlsError::TrustStore(format!("failed to load {path}: {e}")));
        }
    }
    Err(TlsError::TrustStore(
        "no supported Linux system CA bundle found".into(),
    ))
}

#[cfg(windows)]
fn windows_roots() -> Result<Vec<Vec<u8>>, TlsError> {
    crate::tls::windows_trust::load_system_roots()
        .map_err(|e| TlsError::TrustStore(format!("failed to open Windows system ROOT store: {e}")))
}

#[cfg(windows)]
fn bridge_windows_roots(
    store: &mut leyline_bssl::x509::store::X509StoreBuilderRef,
    roots: &[Vec<u8>],
    mut skip: impl FnMut(SkipReason, leyline_bssl::error::ErrorStack),
) -> (usize, usize) {
    let mut loaded = 0usize;
    let mut skipped = 0usize;
    for der in roots {
        let added = X509::from_der(der)
            .map_err(|e| (SkipReason::Parse, e))
            .and_then(|cert| store.add_cert(cert).map_err(|e| (SkipReason::Store, e)));
        match added {
            Ok(()) => loaded += 1,
            Err((reason, e)) => {
                skipped += 1;
                skip(reason, e);
            }
        }
    }
    (loaded, skipped)
}

#[cfg(windows)]
enum SkipReason {
    Parse,
    Store,
}

#[cfg(windows)]
fn wire_windows_system_trust(builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    let roots = windows_roots()?;
    let (loaded, skipped) =
        bridge_windows_roots(builder.cert_store_mut(), &roots, |reason, e| match reason {
            SkipReason::Store => tracing::debug!(
                target: "leyline::tls::trust",
                err = %e,
                "Windows ROOT cert rejected by BoringSSL store"
            ),
            SkipReason::Parse => tracing::debug!(
                target: "leyline::tls::trust",
                err = %e,
                "Windows ROOT cert failed DER parse"
            ),
        });

    if loaded == 0 {
        return Err(TlsError::TrustStore(format!(
            "Windows system ROOT store bridged zero certificates ({skipped} skipped)"
        )));
    }
    tracing::info!(
        target: "leyline::tls::trust",
        loaded,
        skipped,
        "Windows system ROOT store bridged into BoringSSL"
    );
    Ok(())
}
