use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::config::Config;

const ARCHIVES: [(&str, &str); 2] = [
    ("ssl", "leyline_bssl_ssl"),
    ("crypto", "leyline_bssl_crypto"),
];

fn archive_file(config: &Config, name: &str) -> String {
    if config.target_env == "msvc" {
        format!("{name}.lib")
    } else {
        format!("lib{name}.a")
    }
}

fn find_archive(search_dirs: &[PathBuf], file: &str) -> io::Result<PathBuf> {
    search_dirs
        .iter()
        .map(|dir| dir.join(file))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("BoringSSL archive {file} not found in {search_dirs:?}"),
            )
        })
}

pub(crate) fn link_renamed(config: &Config, search_dirs: &[PathBuf]) -> io::Result<()> {
    let link_dir: &Path = &config.out_dir.join("leyline-bssl-link");
    fs::create_dir_all(link_dir)?;
    for (source, target) in ARCHIVES {
        let from = find_archive(search_dirs, &archive_file(config, source))?;
        fs::copy(&from, link_dir.join(archive_file(config, target)))?;
        println!("cargo:rerun-if-changed={}", from.display());
    }
    println!("cargo:rustc-link-search=native={}", link_dir.display());
    for (_, target) in ARCHIVES {
        println!("cargo:rustc-link-lib=static={target}");
    }
    Ok(())
}
