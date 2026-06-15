//! Trust-root wiring for BoringSSL `SslContextBuilder`.
//!
//! Split from `builder.rs` so the PEM-env and system-store logic can
//! grow (Windows bridge, planned macOS Keychain bridge) without pushing
//! the profile-application file over its hygiene line cap.
//!
//! Two entry points:
//!
//! - [`wire_env_trust`] honours `SSL_CERT_FILE` and `SSL_CERT_DIR` if
//!   the operator sets them. BoringSSL — unlike OpenSSL — does not
//!   consume these env vars automatically; wiring them here matches
//!   the behaviour operators get from curl / reqwest / Python requests.
//! - [`wire_system_trust`] is the fallback when no env override was
//!   honoured. Dispatches to the platform-specific bridge (Windows
//!   cert store) or to BoringSSL's `set_default_verify_paths` on Unix.

use std::path::PathBuf;

use btls::ssl::{SslAlert, SslContextBuilder, SslFiletype, SslVerifyError, SslVerifyMode};
use btls::x509::{X509StoreContext, X509};
use sha2::{Digest, Sha256};

use crate::tls::error::TlsError;

/// TLS trust and client-certificate configuration.
///
/// Defaults match Leyline's existing behavior: honour `SSL_CERT_FILE` /
/// `SSL_CERT_DIR` when present, otherwise load the platform system roots.
/// Explicit roots are additive, so callers can trust a private CA without
/// losing the normal public web PKI.
#[derive(Debug, Clone)]
pub struct TlsTrustConfig {
    use_env_roots: bool,
    use_system_roots: bool,
    ca_files: Vec<PathBuf>,
    ca_der: Vec<Vec<u8>>,
    client_identity: Option<ClientIdentity>,
    pinned_leaf_sha256: Vec<[u8; 32]>,
}

impl Default for TlsTrustConfig {
    fn default() -> Self {
        Self {
            use_env_roots: true,
            use_system_roots: true,
            ca_files: Vec::new(),
            ca_der: Vec::new(),
            client_identity: None,
            pinned_leaf_sha256: Vec::new(),
        }
    }
}

impl TlsTrustConfig {
    /// Create a trust config with Leyline's default env/system root behavior.
    pub fn new() -> Self {
        Self::default()
    }

    /// Do not honour `SSL_CERT_FILE` / `SSL_CERT_DIR`.
    pub fn without_env_roots(mut self) -> Self {
        self.use_env_roots = false;
        self
    }

    /// Do not load platform system roots.
    pub fn without_system_roots(mut self) -> Self {
        self.use_system_roots = false;
        self
    }

    /// Add a PEM CA file or bundle to the trust store.
    pub fn add_ca_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.ca_files.push(path.into());
        self
    }

    /// Add a DER-encoded CA certificate to the trust store.
    pub fn add_ca_der(mut self, der: impl Into<Vec<u8>>) -> Self {
        self.ca_der.push(der.into());
        self
    }

    /// Add a SHA-256 pin for the DER-encoded leaf certificate.
    ///
    /// Pinning is additive to normal certificate validation: the chain
    /// must still verify against the configured roots, and the leaf
    /// certificate must match one of the configured hashes.
    pub fn add_pinned_leaf_sha256(mut self, sha256: [u8; 32]) -> Self {
        self.pinned_leaf_sha256.push(sha256);
        self
    }

    /// Use a PEM client certificate chain and private key for mTLS.
    pub fn client_identity_files(
        mut self,
        certificate_chain_file: impl Into<PathBuf>,
        private_key_file: impl Into<PathBuf>,
    ) -> Self {
        self.client_identity = Some(ClientIdentity {
            certificate_chain_file: certificate_chain_file.into(),
            private_key_file: private_key_file.into(),
        });
        self
    }

    /// Whether `SSL_CERT_FILE` / `SSL_CERT_DIR` are honoured.
    pub fn uses_env_roots(&self) -> bool {
        self.use_env_roots
    }

    /// Whether platform system roots are loaded.
    pub fn uses_system_roots(&self) -> bool {
        self.use_system_roots
    }

    /// Additional PEM CA files configured by the caller.
    pub fn ca_files(&self) -> &[PathBuf] {
        &self.ca_files
    }

    /// Number of in-memory DER CA certificates configured by the caller.
    pub fn ca_der_count(&self) -> usize {
        self.ca_der.len()
    }

    /// Configured mTLS identity, if any.
    pub fn client_identity(&self) -> Option<&ClientIdentity> {
        self.client_identity.as_ref()
    }

    /// SHA-256 leaf certificate pins.
    pub fn pinned_leaf_sha256(&self) -> &[[u8; 32]] {
        &self.pinned_leaf_sha256
    }

    /// In-memory DER CA certificates configured by the caller (rustls bridge).
    #[cfg(feature = "tls-rustls")]
    pub(crate) fn ca_der(&self) -> &[Vec<u8>] {
        &self.ca_der
    }
}

/// PEM client identity used for mutual TLS.
#[derive(Debug, Clone)]
pub struct ClientIdentity {
    /// PEM certificate chain sent to the server.
    pub certificate_chain_file: PathBuf,
    /// PEM private key matching the leaf certificate.
    pub private_key_file: PathBuf,
}

/// Honour `SSL_CERT_FILE` / `SSL_CERT_DIR` if set. Returns `true` when
/// at least one trust root was loaded from the environment, in which
/// case the caller should skip the system-store fallback.
///
/// On error (missing file, unreadable dir, malformed PEM) the function
/// logs at `warn` and returns `false` so the caller falls back to the
/// system trust store. A failed env override must not abort TLS setup
/// silently — operators need to see the warning — but it also must not
/// leave the process with zero CAs.
pub(crate) fn wire_env_trust(builder: &mut SslContextBuilder) -> bool {
    let mut env_trust_wired = false;
    if let Ok(file) = std::env::var("SSL_CERT_FILE") {
        let file = file.trim();
        if !file.is_empty() {
            match builder.set_ca_file(file) {
                Ok(()) => {
                    env_trust_wired = true;
                    // `warn` level: an env-var overriding the trust
                    // anchor is a security-relevant decision, and
                    // `debug` is off in most deployments. Operators
                    // need to see this unconditionally so a leaked
                    // env-var attack doesn't fly under the radar.
                    tracing::warn!(
                        target: "leyline::tls::trust",
                        ca_file = %file,
                        "SSL_CERT_FILE honoured — process trust store overridden by environment"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        target: "leyline::tls::trust",
                        ca_file = %file,
                        err = %e,
                        "SSL_CERT_FILE could not be loaded; falling back to system trust"
                    );
                }
            }
        }
    }
    if let Ok(dir) = std::env::var("SSL_CERT_DIR") {
        let dir = dir.trim();
        if !dir.is_empty() {
            let dir_path = std::path::PathBuf::from(dir);
            if dir_path.is_dir() {
                // `collect_ca_dir_candidates` filters by extension and
                // follows symlinks (see its docs). Per-file parse
                // errors degrade to debug-log; an all-bad dir falls
                // back to system trust so we never run with zero CAs.
                let candidates = collect_ca_dir_candidates(&dir_path);
                if !candidates.is_empty() || dir_path.exists() {
                    let mut loaded = 0usize;
                    for p in &candidates {
                        match builder.set_ca_file(p) {
                            Ok(()) => loaded += 1,
                            Err(e) => {
                                tracing::debug!(
                                    target: "leyline::tls::trust",
                                    ca_file = %p.display(),
                                    err = %e,
                                    "SSL_CERT_DIR entry skipped"
                                );
                            }
                        }
                    }
                    if loaded > 0 {
                        env_trust_wired = true;
                        tracing::warn!(
                            target: "leyline::tls::trust",
                            ca_dir = %dir,
                            files_loaded = loaded,
                            "SSL_CERT_DIR honoured — process trust store overridden by environment"
                        );
                    } else {
                        tracing::warn!(
                            target: "leyline::tls::trust",
                            ca_dir = %dir,
                            "SSL_CERT_DIR contained no loadable certificates; falling back to system trust"
                        );
                    }
                }
            } else {
                tracing::warn!(
                    target: "leyline::tls::trust",
                    ca_dir = %dir,
                    "SSL_CERT_DIR does not exist; falling back to system trust"
                );
            }
        }
    }
    env_trust_wired
}

/// Apply Leyline's configured trust behavior to an SSL context.
pub(crate) fn wire_configured_trust(
    builder: &mut SslContextBuilder,
    config: &TlsTrustConfig,
) -> Result<(), TlsError> {
    let env_trust_wired = config.use_env_roots && wire_env_trust(builder);
    if config.use_system_roots && !env_trust_wired {
        wire_system_trust(builder)?;
    }

    for path in &config.ca_files {
        builder.set_ca_file(path)?;
    }

    if !config.ca_der.is_empty() {
        let store = builder.cert_store_mut();
        for der in &config.ca_der {
            store.add_cert(X509::from_der(der)?)?;
        }
    }

    if let Some(identity) = &config.client_identity {
        builder.set_certificate_chain_file(&identity.certificate_chain_file)?;
        builder.set_private_key_file(&identity.private_key_file, SslFiletype::PEM)?;
    }

    if !config.pinned_leaf_sha256.is_empty() {
        let pins = config.pinned_leaf_sha256.clone();
        builder.set_custom_verify_callback(SslVerifyMode::PEER, move |ssl| {
            let store = ssl.ssl_context().cert_store();
            let cert = ssl
                .peer_certificate()
                .ok_or(SslVerifyError::Invalid(SslAlert::CERTIFICATE_UNKNOWN))?;
            let chain = ssl
                .peer_cert_chain()
                .ok_or(SslVerifyError::Invalid(SslAlert::CERTIFICATE_UNKNOWN))?;

            let chain_ok = X509StoreContext::new()
                .map_err(|_| SslVerifyError::Invalid(SslAlert::INTERNAL_ERROR))?
                .init(store, &cert, chain, |store_ctx| {
                    let verified = store_ctx.verify_cert()?;
                    Ok(verified && store_ctx.verify_result().is_ok())
                })
                .map_err(|_| SslVerifyError::Invalid(SslAlert::INTERNAL_ERROR))?;
            if !chain_ok {
                return Err(SslVerifyError::Invalid(SslAlert::UNKNOWN_CA));
            }

            let der = cert
                .to_der()
                .map_err(|_| SslVerifyError::Invalid(SslAlert::INTERNAL_ERROR))?;
            let digest: [u8; 32] = Sha256::digest(&der).into();
            if pins.iter().any(|pin| pin == &digest) {
                Ok(())
            } else {
                Err(SslVerifyError::Invalid(SslAlert::CERTIFICATE_UNKNOWN))
            }
        });
    }

    Ok(())
}

/// Load the platform's system trust store into the builder's
/// `X509_STORE`.
///
/// Linux: BoringSSL's `set_default_verify_paths` points at the
/// canonical `/etc/ssl/certs` / `/etc/pki/tls/certs` locations shipped
/// with every mainstream distro.
///
/// macOS: BoringSSL's compiled-in paths point at a Homebrew-style
/// `/usr/local/etc/openssl/` that does not exist on a default macOS
/// install, so `set_default_verify_paths` silently yields zero roots.
/// We try the OpenSSL-compat bundle at `/etc/ssl/cert.pem` (shipped by
/// Apple since 10.13, rebuilt from the System Keychain on every OS
/// update), falling back to the default-paths call as a last resort
/// so a non-standard macOS layout still loads *something*.
///
/// Windows: BoringSSL's default paths point at Unix-style directories
/// that do not exist, so without a bridge the process ends up with
/// zero trust roots and every HTTPS handshake fails with
/// `unable to get local issuer certificate`. We pull every cert from
/// the logical `"ROOT"` Windows store via the Win32 crypto API (see
/// `windows_trust` module) and push each one into the BoringSSL store.
///
/// Returns `TlsError` only when the platform bridge itself fails
/// catastrophically. Individual malformed roots are skipped with a
/// `debug` log — a single bad cert must not void the whole trust store.
pub(crate) fn wire_system_trust(builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    #[cfg(windows)]
    {
        wire_windows_system_trust(builder)
    }
    #[cfg(target_os = "macos")]
    {
        wire_macos_system_trust(builder)
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        builder.set_default_verify_paths()?;
        Ok(())
    }
}

/// macOS-only: load Apple's rebuilt-on-every-update OpenSSL bundle at
/// `/etc/ssl/cert.pem`, which mirrors the System Keychain roots. Falls
/// back to `set_default_verify_paths` if the file is missing so an
/// unusual macOS layout still loads *something* instead of silently
/// leaving the process with zero trust anchors.
///
/// We deliberately do not bind `security-framework` / the Keychain
/// APIs directly: `/etc/ssl/cert.pem` already holds the same trust set
/// and is rebuilt by the OS, so the binding would add a Foundation
/// runtime dependency and a round-trip through CF for no material
/// upside over reading a PEM file.
#[cfg(target_os = "macos")]
fn wire_macos_system_trust(builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    const APPLE_BUNDLE: &str = "/etc/ssl/cert.pem";
    if std::path::Path::new(APPLE_BUNDLE).exists() {
        match builder.set_ca_file(APPLE_BUNDLE) {
            Ok(()) => {
                tracing::info!(
                    target: "leyline::tls::trust",
                    path = APPLE_BUNDLE,
                    "macOS system trust loaded from Apple OpenSSL-compat bundle"
                );
                return Ok(());
            }
            Err(e) => {
                tracing::warn!(
                    target: "leyline::tls::trust",
                    path = APPLE_BUNDLE,
                    err = %e,
                    "macOS system trust bundle parse failed; falling back to default paths"
                );
            }
        }
    }
    builder.set_default_verify_paths()?;
    Ok(())
}

/// Windows-only: enumerate the `"ROOT"` system store via Win32 crypto
/// API and push each DER-encoded cert into the BoringSSL `X509_STORE`.
///
/// Mirrors what `rustls-native-certs` does, minus the `schannel` crate
/// dependency that `deny.toml` bans alongside other alternative TLS
/// backends.
#[cfg(windows)]
fn wire_windows_system_trust(builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    use btls::x509::X509;

    let roots = match crate::tls::windows_trust::load_system_roots() {
        Ok(r) => r,
        Err(e) => {
            // A connector with no roots fails every handshake later with an
            // opaque cert error; fail at build time instead of swallowing.
            return Err(TlsError::TrustStore(format!(
                "failed to open Windows system ROOT store: {e}"
            )));
        }
    };

    let store = builder.cert_store_mut();
    let mut loaded = 0usize;
    let mut skipped = 0usize;
    for der in &roots {
        match X509::from_der(der) {
            Ok(cert) => match store.add_cert(cert) {
                Ok(()) => loaded += 1,
                Err(e) => {
                    skipped += 1;
                    tracing::debug!(
                        target: "leyline::tls::trust",
                        err = %e,
                        "Windows ROOT cert rejected by BoringSSL store"
                    );
                }
            },
            Err(e) => {
                skipped += 1;
                tracing::debug!(
                    target: "leyline::tls::trust",
                    err = %e,
                    "Windows ROOT cert failed DER parse"
                );
            }
        }
    }

    if loaded == 0 {
        // Empty store + PEER verify mode = every handshake fails opaquely.
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

/// Walk `dir` and return every entry that passes the CA-dir
/// acceptance rules:
///
/// - filename extension matches `.pem` / `.crt` / `.cer` (ASCII
///   case-insensitive),
/// - resolving the path through [`std::fs::metadata`] (which DOES
///   follow symlinks) yields a regular file.
///
/// Broken symlinks, symlink loops, directories, and targets with
/// unrecognised extensions are skipped silently. Returns entries
/// in whatever order `read_dir` yields — callers that require
/// determinism must sort.
///
/// Extracted so the dir-walk semantics can be covered by regression
/// tests without needing a live `SslContextBuilder`.
pub(crate) fn collect_ca_dir_candidates(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        let ext_ok = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e.to_ascii_lowercase().as_str(), "pem" | "crt" | "cer"))
            .unwrap_or(false);
        if !ext_ok {
            continue;
        }
        // `fs::metadata` follows symlinks — crucial for the
        // Debian/Ubuntu/RHEL `/etc/ssl/certs` layout which is
        // entirely symlinks pointing into
        // `/usr/share/ca-certificates/`. A previous version had a bug
        // where `DirEntry::metadata()` (non-following) rejected
        // every symlink and left the process with zero CAs.
        let Ok(resolved) = std::fs::metadata(&p) else {
            continue;
        };
        if !resolved.is_file() {
            continue;
        }
        out.push(p);
    }
    out
}

#[cfg(test)]
mod ca_dir_tests {
    //! Coverage gate: SSL_CERT_DIR regression (SSL_CERT_DIR silently
    //! disabled trust on Debian/Ubuntu/RHEL because all entries
    //! in `/etc/ssl/certs` are symlinks) shipped without a test.
    //! These gates guard the symlink-following, extension-filter,
    //! and file-type-after-resolve semantics directly. If a future
    //! refactor re-introduces `DirEntry::metadata()` or drops the
    //! symlink-follow, these tests fail.

    use super::collect_ca_dir_candidates;
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Disposable per-test directory under the OS temp dir. Named
    /// with a process-unique counter so parallel tests never clash.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!(
                "leyline-ca-{}-{}-{}",
                label,
                std::process::id(),
                id
            ));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_pem(path: &Path, label: &str) {
        let mut f = fs::File::create(path).unwrap();
        // Content is arbitrary — the helper only walks the dir;
        // it does NOT call `set_ca_file`.
        writeln!(
            f,
            "-----BEGIN CERTIFICATE-----\n{label}\n-----END CERTIFICATE-----"
        )
        .unwrap();
    }

    #[test]
    fn missing_dir_yields_empty_candidates() {
        let tmp = TempDir::new("missing");
        let nowhere = tmp.path().join("does-not-exist");
        assert!(collect_ca_dir_candidates(&nowhere).is_empty());
    }

    #[test]
    fn accepts_regular_pem_crt_cer_files() {
        let tmp = TempDir::new("regular");
        write_pem(&tmp.path().join("a.pem"), "a");
        write_pem(&tmp.path().join("b.crt"), "b");
        write_pem(&tmp.path().join("c.cer"), "c");
        let mut v = collect_ca_dir_candidates(tmp.path());
        v.sort();
        assert_eq!(v.len(), 3, "expected 3 candidates, got {v:?}");
    }

    #[test]
    fn extension_match_is_case_insensitive() {
        let tmp = TempDir::new("case");
        write_pem(&tmp.path().join("a.PEM"), "a");
        write_pem(&tmp.path().join("b.Crt"), "b");
        let v = collect_ca_dir_candidates(tmp.path());
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn wrong_extensions_skipped() {
        let tmp = TempDir::new("ext");
        write_pem(&tmp.path().join("a.pem"), "a");
        write_pem(&tmp.path().join("b.hash"), "b");
        write_pem(&tmp.path().join("README"), "c");
        write_pem(&tmp.path().join("d.txt"), "d");
        write_pem(&tmp.path().join("e.pem.bak"), "e");
        let v = collect_ca_dir_candidates(tmp.path());
        assert_eq!(v.len(), 1, "only a.pem should pass: {v:?}");
    }

    #[test]
    #[cfg(unix)]
    fn symlink_to_regular_file_is_accepted() {
        // This is the bug: the Debian/Ubuntu `/etc/ssl/certs`
        // layout is ENTIRELY symlinks. If this test ever regresses,
        // every mainstream Linux distro loses TLS trust.
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new("sym");
        let real = tmp.path().join("real.pem");
        write_pem(&real, "real");
        let link = tmp.path().join("link.pem");
        symlink(&real, &link).unwrap();
        let v = collect_ca_dir_candidates(tmp.path());
        // Both the real file and the symlink pointing at it are
        // loadable candidates — BoringSSL deduplicates by subject
        // so double-loading is harmless.
        assert_eq!(v.len(), 2, "got {v:?}");
    }

    #[test]
    #[cfg(unix)]
    fn broken_symlink_skipped() {
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new("broken");
        let link = tmp.path().join("dangling.pem");
        symlink("/nonexistent/target.pem", &link).unwrap();
        let v = collect_ca_dir_candidates(tmp.path());
        assert!(v.is_empty(), "broken symlink must be skipped: {v:?}");
    }

    #[test]
    fn directory_entry_with_cert_extension_skipped() {
        let tmp = TempDir::new("subdir");
        // `foo.pem/` as a directory must not be treated as a cert.
        fs::create_dir(tmp.path().join("bogus.pem")).unwrap();
        write_pem(&tmp.path().join("real.pem"), "real");
        let v = collect_ca_dir_candidates(tmp.path());
        assert_eq!(v.len(), 1, "only real.pem expected: {v:?}");
    }
}
