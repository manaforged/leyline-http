use leyline_bssl::ssl::SslContextBuilder;

use super::LoadedTrust;
use super::files::add_root_file;

pub(super) fn wire_env_trust(builder: &mut SslContextBuilder, trust: &mut LoadedTrust) -> bool {
    let mut loaded_any = false;
    if let Ok(file) = std::env::var("SSL_CERT_FILE") {
        let file = file.trim();
        if !file.is_empty() {
            match add_root_file(builder, std::path::Path::new(file), trust) {
                Ok(_) => {
                    loaded_any = true;
                    tracing::warn!(
                        target: "leyline::tls::trust",
                        ca_file = %file,
                        "SSL_CERT_FILE honoured — environment trust root added"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        target: "leyline::tls::trust",
                        ca_file = %file,
                        err = %e,
                        "SSL_CERT_FILE could not be loaded"
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
                let candidates = collect_ca_dir_candidates(&dir_path);
                if !candidates.is_empty() || dir_path.exists() {
                    let mut loaded = 0usize;
                    for p in &candidates {
                        match add_root_file(builder, p, trust) {
                            Ok(_) => loaded += 1,
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
                        loaded_any = true;
                        tracing::warn!(
                            target: "leyline::tls::trust",
                            ca_dir = %dir,
                            files_loaded = loaded,
                            "SSL_CERT_DIR honoured — environment trust roots added"
                        );
                    } else {
                        tracing::warn!(
                            target: "leyline::tls::trust",
                            ca_dir = %dir,
                            "SSL_CERT_DIR contained no loadable certificates"
                        );
                    }
                }
            } else {
                tracing::warn!(
                    target: "leyline::tls::trust",
                    ca_dir = %dir,
                    "SSL_CERT_DIR does not exist"
                );
            }
        }
    }
    loaded_any
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
