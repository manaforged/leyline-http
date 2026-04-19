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

use btls::ssl::SslContextBuilder;

use crate::tls::error::TlsError;

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

/// Load the platform's system trust store into the builder's
/// `X509_STORE`.
///
/// Unix: BoringSSL's `set_default_verify_paths` points at the
/// canonical `/etc/ssl/certs` / `/etc/pki/tls/certs` locations shipped
/// with every mainstream distro and macOS (via OpenSSL compat dirs).
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
    #[cfg(not(windows))]
    {
        builder.set_default_verify_paths()?;
        Ok(())
    }
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
            tracing::error!(
                target: "leyline::tls::trust",
                err = %e,
                "failed to open Windows system ROOT store; HTTPS requests will fail"
            );
            return Ok(());
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
        tracing::error!(
            target: "leyline::tls::trust",
            skipped,
            "Windows system ROOT store bridged zero certificates; HTTPS requests will fail"
        );
    } else {
        tracing::info!(
            target: "leyline::tls::trust",
            loaded,
            skipped,
            "Windows system ROOT store bridged into BoringSSL"
        );
    }
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
