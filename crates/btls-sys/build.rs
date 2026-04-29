use std::env;
use std::path::{Path, PathBuf};

const SUPPORTED_TARGET: &str = "x86_64-pc-windows-msvc";

fn main() {
    println!("cargo:rerun-if-env-changed=BORING_BSSL_PATH");
    println!("cargo:rerun-if-env-changed=BORING_BSSL_RUST_CPPLIB");

    let target = env::var("TARGET").expect("Cargo should set TARGET");
    if target != SUPPORTED_TARGET {
        panic!(
            "Leyline's local btls-sys shim has prebuilt BoringSSL artifacts only for {SUPPORTED_TARGET}; target {target} still needs a pregenerated binding/native-lib bundle."
        );
    }

    if env::var_os("CARGO_FEATURE_FIPS").is_some() {
        panic!("The local Leyline btls-sys shim does not package FIPS BoringSSL artifacts.");
    }

    if env::var_os("CARGO_FEATURE_MLKEM").is_some() {
        panic!(
            "The local Leyline btls-sys shim does not package ML-KEM-specific BoringSSL artifacts."
        );
    }

    if env::var_os("CARGO_FEATURE_PREFIX_SYMBOLS").is_some() {
        println!("cargo:warning=btls-sys prefix-symbols is ignored by Leyline's prebuilt Windows/MSVC shim.");
    }

    let lib_dir = match env::var_os("BORING_BSSL_PATH") {
        Some(path) => find_lib_dir(PathBuf::from(path), &target),
        None => {
            let manifest_dir = PathBuf::from(
                env::var_os("CARGO_MANIFEST_DIR").expect("Cargo should set CARGO_MANIFEST_DIR"),
            );
            manifest_dir
                .join("native")
                .join(SUPPORTED_TARGET)
                .join("lib")
        }
    };

    assert_static_libs_exist(&lib_dir, &target);

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    if let Some(cpp_lib) = env::var_os("BORING_BSSL_RUST_CPPLIB").and_then(|v| v.into_string().ok())
    {
        println!("cargo:rustc-link-lib={cpp_lib}");
    }
    println!("cargo:rustc-link-lib=static=crypto");
    println!("cargo:rustc-link-lib=static=ssl");
    println!("cargo:rustc-link-lib=advapi32");
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
