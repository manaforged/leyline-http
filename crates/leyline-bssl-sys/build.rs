use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const SUPPORTED_TARGETS: &[&str] = &[
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "aarch64-unknown-linux-gnu",
];

fn main() {
    println!("cargo:rerun-if-env-changed=BORING_BSSL_PATH");
    println!("cargo:rerun-if-env-changed=BORING_BSSL_RUST_CPPLIB");

    let target = env::var("TARGET").expect("Cargo should set TARGET");
    if !SUPPORTED_TARGETS.iter().any(|t| *t == target) && env::var_os("BORING_BSSL_PATH").is_none()
    {
        panic!(
            "\n\
             leyline-bssl-sys ships prebuilt BoringSSL only for {SUPPORTED_TARGETS:?},\n\
             and the current target `{target}` is not one of them.\n\n\
             To build for `{target}`, pick one:\n\
               1. Point BORING_BSSL_PATH at a BoringSSL build for this target, e.g.\n\
                    BORING_BSSL_PATH=/path/to/boringssl cargo build\n\
               2. Build the carried BoringSSL from source for this target via\n\
                  scripts/package-bssl.sh (needs CMake, Perl, libclang, Go),\n\
                  then commit the resulting native/<target>/lib + src/bindings/<target>.rs.\n"
        );
    }

    if env::var_os("CARGO_FEATURE_FIPS").is_some() {
        panic!("leyline-bssl-sys does not package FIPS BoringSSL artifacts.");
    }

    if env::var_os("CARGO_FEATURE_MLKEM").is_some() {
        panic!("leyline-bssl-sys does not package ML-KEM-specific BoringSSL artifacts.");
    }

    let lib_dir = match env::var_os("BORING_BSSL_PATH") {
        Some(path) => find_lib_dir(PathBuf::from(path), &target),
        None => {
            let manifest_dir = PathBuf::from(
                env::var_os("CARGO_MANIFEST_DIR").expect("Cargo should set CARGO_MANIFEST_DIR"),
            );
            manifest_dir.join("native").join(&target).join("lib")
        }
    };

    assert_static_libs_exist(&lib_dir, &target);

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo should set OUT_DIR"));
    for name in ["crypto", "ssl"] {
        let source = lib_dir.join(static_lib_file(name, &target));
        let linked_name = format!("leyline_{name}");
        let destination = out_dir.join(static_lib_file(&linked_name, &target));
        fs::copy(&source, &destination).unwrap_or_else(|error| {
            panic!("Failed to stage {}: {error}", source.display());
        });
        println!("cargo:rerun-if-changed={}", source.display());
        println!("cargo:rustc-link-lib=static={linked_name}");
    }
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    if let Some(cpp_lib) = env::var_os("BORING_BSSL_RUST_CPPLIB").and_then(|v| v.into_string().ok())
    {
        println!("cargo:rustc-link-lib={cpp_lib}");
    }
    if target.ends_with("msvc") {
        println!("cargo:rustc-link-lib=advapi32");
    } else if target.contains("apple") {
        // macOS: BoringSSL pulls in libc++ (Apple's default) and pthreads
        // are part of libSystem so no explicit pthread link needed.
        println!("cargo:rustc-link-lib=c++");
    } else {
        // Linux: BoringSSL needs libstdc++ (or libc++) for the small amount
        // of C++ inside it, and pthread for sync primitives.
        println!("cargo:rustc-link-lib=stdc++");
        println!("cargo:rustc-link-lib=pthread");
    }
}

fn find_lib_dir(root: PathBuf, target: &str) -> PathBuf {
    let candidates = [
        root.join("lib"),
        root.join("crypto"),
        root.join("ssl"),
        root.clone(),
    ];

    for candidate in candidates {
        if has_static_libs(&candidate, target) {
            return candidate;
        }
    }

    root
}

fn assert_static_libs_exist(lib_dir: &Path, target: &str) {
    if has_static_libs(lib_dir, target) {
        return;
    }

    panic!(
        "BoringSSL static libs were not found in {}. Expected {} and {}.",
        lib_dir.display(),
        static_lib_file("crypto", target),
        static_lib_file("ssl", target),
    );
}

fn has_static_libs(lib_dir: &Path, target: &str) -> bool {
    lib_dir.join(static_lib_file("crypto", target)).exists()
        && lib_dir.join(static_lib_file("ssl", target)).exists()
}

fn static_lib_file(name: &str, target: &str) -> String {
    if target.ends_with("msvc") {
        format!("{name}.lib")
    } else {
        format!("lib{name}.a")
    }
}
