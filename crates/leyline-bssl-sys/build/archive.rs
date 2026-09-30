use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::cmake::build_boringssl_or_get_prebuilt;
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

fn link_renamed(config: &Config, search_dirs: &[PathBuf]) -> io::Result<()> {
    let link_dir: &Path = &config.out_dir.join("leyline-bssl-link");
    fs::create_dir_all(link_dir)?;
    for (source, target) in ARCHIVES {
        let from = find_archive(search_dirs, &archive_file(config, source))?;
        fs::copy(&from, link_dir.join(archive_file(config, target)))?;
        if !from.starts_with(&config.out_dir) {
            println!("cargo:rerun-if-changed={}", from.display());
        }
    }
    println!("cargo:rustc-link-search=native={}", link_dir.display());
    for (_, target) in ARCHIVES {
        println!("cargo:rustc-link-lib=static={target}");
    }
    Ok(())
}

fn msvc_lib_subdir(config: &Config) -> Option<&'static str> {
    if config.target.ends_with("-msvc") {
        let debug_env_var = config
            .env
            .debug
            .as_ref()
            .expect("DEBUG variable not defined in env");

        let deb_info = match debug_env_var.to_str() {
            Some("false") => false,
            Some("true") => true,
            _ => panic!("Unknown DEBUG={debug_env_var:?} env var."),
        };

        let opt_env_var = config
            .env
            .opt_level
            .as_ref()
            .expect("OPT_LEVEL variable not defined in env");

        let subdir = match opt_env_var.to_str() {
            Some("0") => "Debug",
            Some("1" | "2" | "3") => {
                if deb_info {
                    "RelWithDebInfo"
                } else {
                    "Release"
                }
            }
            Some("s" | "z") => "MinSizeRel",
            _ => panic!("Unknown OPT_LEVEL={opt_env_var:?} env var."),
        };

        Some(subdir)
    } else {
        None
    }
}

fn get_cpp_runtime_lib(config: &Config) -> Option<String> {
    if let Some(ref cpp_lib) = config.env.cpp_runtime_lib {
        return cpp_lib.clone().into_string().ok();
    }

    match &*config.target_os {
        "macos" | "ios" | "tvos" | "freebsd" | "openbsd" | "android" => Some("c++".into()),
        _ if config.unix || config.target_env == "gnu" => Some("stdc++".into()),
        _ => None,
    }
}

pub(crate) fn emit_link_directives(config: &Config) -> io::Result<()> {
    let bssl_dir = build_boringssl_or_get_prebuilt(config);
    let msvc_lib_subdir = msvc_lib_subdir(config);

    let subdirs = if config.is_bazel {
        &["lib"][..]
    } else {
        &["lib", "crypto", "ssl", ""][..]
    };

    let search_dirs: Vec<PathBuf> = subdirs
        .iter()
        .map(|subdir| {
            let dir = bssl_dir.join(subdir);
            msvc_lib_subdir
                .map(|s| dir.join(s))
                .filter(|d| d.exists())
                .unwrap_or(dir)
        })
        .collect();
    link_renamed(config, &search_dirs)?;

    if let Some(cpp_lib) = get_cpp_runtime_lib(config) {
        println!("cargo:rustc-link-lib={cpp_lib}");
    }

    if config.target_os == "windows" {
        println!("cargo:rustc-link-lib=advapi32");
    }
    Ok(())
}
