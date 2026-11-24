use leyline_bssl::ssl::SslContextBuilder;

use super::TrustKey;
use super::config::Pem;
use super::files::add_root;

fn non_empty_env(name: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

pub(super) fn env_roots_set() -> bool {
    ["SSL_CERT_FILE", "SSL_CERT_DIR"]
        .into_iter()
        .any(|name| non_empty_env(name).is_some())
}

pub(super) fn wire_env_trust(builder: &mut SslContextBuilder, trust: &mut TrustKey) -> bool {
    let from_file =
        non_empty_env("SSL_CERT_FILE").is_some_and(|file| load_env_file(builder, trust, &file));
    let from_dir =
        non_empty_env("SSL_CERT_DIR").is_some_and(|dir| load_env_dir(builder, trust, &dir));
    from_file || from_dir
}

fn load_env_file(builder: &mut SslContextBuilder, trust: &mut TrustKey, file: &str) -> bool {
    match add_root(builder, &Pem::File(file.into()), trust) {
        Ok(_) => {
            tracing::warn!(
                target: "leyline::tls::trust",
                ca_file = %file,
                "SSL_CERT_FILE honoured — environment trust root added"
            );
            true
        }
        Err(e) => {
            tracing::warn!(
                target: "leyline::tls::trust",
                ca_file = %file,
                err = %e,
                "SSL_CERT_FILE could not be loaded"
            );
            false
        }
    }
}

fn load_env_dir(builder: &mut SslContextBuilder, trust: &mut TrustKey, dir: &str) -> bool {
    let dir_path = std::path::PathBuf::from(dir);
    if !dir_path.is_dir() {
        tracing::warn!(
            target: "leyline::tls::trust",
            ca_dir = %dir,
            "SSL_CERT_DIR does not exist"
        );
        return false;
    }
    let mut loaded = 0usize;
    for path in collect_ca_dir_candidates(&dir_path) {
        match add_root(builder, &Pem::File(path.clone()), trust) {
            Ok(_) => loaded += 1,
            Err(e) => {
                tracing::debug!(
                    target: "leyline::tls::trust",
                    ca_file = %path.display(),
                    err = %e,
                    "SSL_CERT_DIR entry skipped"
                );
            }
        }
    }
    if loaded > 0 {
        tracing::warn!(
            target: "leyline::tls::trust",
            ca_dir = %dir,
            files_loaded = loaded,
            "SSL_CERT_DIR honoured — environment trust roots added"
        );
        true
    } else {
        tracing::warn!(
            target: "leyline::tls::trust",
            ca_dir = %dir,
            "SSL_CERT_DIR contained no loadable certificates"
        );
        false
    }
}

pub(super) fn collect_ca_dir_candidates(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
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
