use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::config::Config;
use crate::prefix::PrefixCallback;
use crate::process::run_command;
use crate::source::get_boringssl_source_path;
use crate::targets::{get_apple_sdk_name, pick_best_android_ndk_toolchain};

fn get_extra_clang_args_for_bindgen(config: &Config) -> Vec<String> {
    let mut params = Vec::new();

    match &*config.target_os {
        "ios" | "macos" | "tvos" => {
            let sdk = get_apple_sdk_name(config);
            match run_command(Command::new("xcrun").args(["--show-sdk-path", "--sdk", sdk])) {
                Ok(output) => {
                    let sysroot = std::str::from_utf8(&output.stdout).expect("xcrun output");
                    params.push("-isysroot".to_string());
                    params.push(sysroot.trim_end().to_string());
                }
                Err(e) => {
                    println!("cargo:warning={e}");
                }
            }
        }
        "android" => {
            let mut android_sysroot = config
                .env
                .android_ndk_home
                .clone()
                .expect("Please set ANDROID_NDK_HOME for Android build");

            android_sysroot.extend(["toolchains", "llvm", "prebuilt"]);

            match pick_best_android_ndk_toolchain(&android_sysroot) {
                Ok(toolchain) => {
                    android_sysroot.push(toolchain);
                    android_sysroot.push("sysroot");
                    params.push("--sysroot".to_string());
                    params.push(android_sysroot.into_os_string().into_string().unwrap());
                }
                Err(e) => {
                    println!("cargo:warning=failed to find prebuilt Android NDK toolchain for bindgen: {e}");
                }
            }
        }
        _ => {}
    }

    params
}

fn check_include_path(path: PathBuf) -> Result<PathBuf, String> {
    if path.join("openssl").join("x509v3.h").exists() {
        Ok(path)
    } else {
        Err(format!(
            "Include path {} {}",
            path.display(),
            if !path.exists() {
                "does not exist"
            } else {
                "does not have expected openssl/x509v3.h"
            }
        ))
    }
}

fn get_include_path(config: &Config) -> Result<PathBuf, String> {
    if let Some(path) = &config.env.include_path {
        return check_include_path(path.to_owned());
    }

    if let Some(bssl_path) = &config.env.path {
        return check_include_path(bssl_path.join("include"));
    }

    let src_path = get_boringssl_source_path(config);
    check_include_path(src_path.join("include"))
        .or_else(|_| check_include_path(src_path.join("src").join("include")))
}

pub(crate) fn generate_bindings(config: &Config) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let include_path = get_include_path(config)?;

    let target_rust_version = bindgen::RustTarget::stable(82, 0)
        .map_err(|e| format!("bindgen does not recognize target rust version: {e}"))?;

    let mut builder = bindgen::Builder::default()
        .rust_target(target_rust_version)
        .derive_copy(true)
        .derive_debug(true)
        .derive_default(true)
        .derive_eq(false)
        .derive_partialeq(false)
        .default_enum_style(bindgen::EnumVariation::NewType {
            is_bitfield: false,
            is_global: false,
        })
        .default_macro_constant_type(bindgen::MacroTypeVariation::Signed)
        .generate_comments(true)
        .fit_macro_constants(false)
        .size_t_is_usize(true)
        .layout_tests(config.env.debug.is_some())
        .merge_extern_blocks(true)
        .prepend_enum_name(true)
        .blocklist_type("max_align_t")
        .clang_args(get_extra_clang_args_for_bindgen(config))
        .clang_arg("-I")
        .clang_arg(include_path.display().to_string());

    if let Some(sysroot) = &config.env.sysroot {
        builder = builder
            .clang_arg("--sysroot")
            .clang_arg(sysroot.display().to_string());

        let target_include_dir = sysroot.join(format!(
            "usr/include/{}-{}-{}",
            config.target_arch, config.target_os, config.target_env
        ));
        if target_include_dir.is_dir() {
            builder = builder
                .clang_arg("-I")
                .clang_arg(target_include_dir.display().to_string());
        }
    }

    builder = builder.parse_callbacks(Box::new(PrefixCallback::read(
        &include_path,
        &config.target_os,
    )?));

    let must_have_headers = [
        "aes.h",
        "asn1_mac.h",
        "asn1t.h",
        "blake2.h",
        "blowfish.h",
        "cast.h",
        "chacha.h",
        "cmac.h",
        "cpu.h",
        "curve25519.h",
        "des.h",
        "dtls1.h",
        "err.h",
        "hkdf.h",
        "hpke.h",
        "ossl_typ.h",
        "pkcs12.h",
        "poly1305.h",
        "x509v3.h",
    ];
    let headers = [
        "hmac.h",
        "hrss.h",
        "md4.h",
        "md5.h",
        "mldsa.h",
        "mlkem.h",
        "obj_mac.h",
        "objects.h",
        "opensslv.h",
        "rand.h",
        "rc4.h",
        "ripemd.h",
        "siphash.h",
        "srtp.h",
        "trust_token.h",
    ];
    for (i, header) in must_have_headers.into_iter().chain(headers).enumerate() {
        let header_path = include_path.join("openssl").join(header);
        if header_path.exists() {
            builder = builder.header(header_path.to_str().unwrap());
        } else {
            let err = format!("'openssl/{header}' is missing from '{}'. The include path may be incorrect or contain an outdated version of OpenSSL/BoringSSL", include_path.display());
            if i < must_have_headers.len() {
                return Err(err.into());
            }
            println!("cargo::warning={err}");
        }
    }

    let bindings = builder.generate()?;
    let mut source_code = Vec::new();
    bindings
        .write(Box::new(&mut source_code))
        .map_err(|e| format!("Couldn't serialize bindings: {e}"))?;
    ensure_err_lib_enum_is_named(&mut source_code);
    let bindings_path = config.out_dir.join("bindings.rs");
    fs::write(&bindings_path, source_code).map_err(|e| {
        format!(
            "Couldn't write bindings to {}: {e}",
            bindings_path.display()
        )
    })?;
    Ok(bindings_path)
}

fn ensure_err_lib_enum_is_named(source_code: &mut Vec<u8>) {
    let src = String::from_utf8_lossy(source_code);
    let enum_type = src
        .split_once("ERR_LIB_SSL:")
        .and_then(|(_, def)| Some(def.split_once('=')?.0))
        .unwrap_or("_bindgen_ty_1");

    source_code.extend_from_slice(
        format!("\n/// Newtype for [`ERR_LIB_SSL`] constants\npub use {enum_type} as ErrLib;\n")
            .as_bytes(),
    );
}
