use std::env;
use std::ffi::OsString;
use std::path::PathBuf;

const SUPPORTED_TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-pc-windows-msvc",
];

pub(crate) struct Config {
    pub(crate) manifest_dir: PathBuf,
    pub(crate) out_dir: PathBuf,
    pub(crate) is_bazel: bool,
    pub(crate) host: String,
    pub(crate) target: String,
    pub(crate) target_arch: String,
    pub(crate) target_os: String,
    pub(crate) unix: bool,
    pub(crate) target_env: String,
    pub(crate) target_features: Vec<String>,
    pub(crate) env: Env,
}

pub(crate) struct Env {
    pub(crate) path: Option<PathBuf>,
    pub(crate) include_path: Option<PathBuf>,
    pub(crate) source_path: Option<PathBuf>,
    pub(crate) assume_patched: bool,
    pub(crate) sysroot: Option<PathBuf>,
    pub(crate) compiler_external_toolchain: Option<PathBuf>,
    pub(crate) debug: Option<OsString>,
    pub(crate) opt_level: Option<OsString>,
    pub(crate) android_ndk_home: Option<PathBuf>,
    pub(crate) cmake_toolchain_file: Option<PathBuf>,
    pub(crate) cpp_runtime_lib: Option<OsString>,
    pub(crate) cc: Option<OsString>,
    pub(crate) cxx: Option<OsString>,
    pub(crate) docs_rs: bool,
}

impl Config {
    pub(crate) fn from_env() -> Result<Self, &'static str> {
        let manifest_dir = env::var_os("CARGO_MANIFEST_DIR")
            .ok_or("CARGO_MANIFEST_DIR")?
            .into();
        let out_dir = env::var_os("OUT_DIR").ok_or("OUT_DIR")?.into();
        let host = env::var("HOST").unwrap();
        let target = env::var("TARGET").unwrap();
        let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
        let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
        let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap();
        let unix = env::var("CARGO_CFG_UNIX").is_ok();

        let target_features = env::var("CARGO_CFG_TARGET_FEATURE")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.to_owned())
            .collect();

        let env = Env::from_env(&host, &target);

        let is_bazel = env
            .source_path
            .as_ref()
            .is_some_and(|path| path.join("src").exists());

        println!(
            "cargo:version_major={}",
            env::var("CARGO_PKG_VERSION_MAJOR").unwrap_or_default()
        );

        let config = Self {
            manifest_dir,
            out_dir,
            is_bazel,
            host,
            target,
            target_arch,
            target_os,
            unix,
            target_env,
            target_features,
            env,
        };

        config.check_feature_compatibility()?;

        Ok(config)
    }

    pub(crate) fn check_supported_target(&self) -> Result<(), String> {
        if SUPPORTED_TARGETS.contains(&self.target.as_str()) {
            return Ok(());
        }
        Err(format!(
            "leyline-bssl-sys supports only {}; the current target `{}` is not one of them",
            SUPPORTED_TARGETS.join(", "),
            self.target
        ))
    }

    fn check_feature_compatibility(&self) -> Result<(), &'static str> {
        let is_precompiled_native_lib = self.env.path.is_some();
        let is_external_native_lib_source =
            !is_precompiled_native_lib && self.env.source_path.is_none();

        if self.env.assume_patched && is_external_native_lib_source {
            return Err(
                "`LEYLINE_BSSL_ASSUME_PATCHED` env variable is supposed to be used with \
                `LEYLINE_BSSL_PATH` or `LEYLINE_BSSL_SOURCE_PATH` env variables",
            );
        }
        Ok(())
    }
}

impl Env {
    fn from_env(host: &str, target: &str) -> Self {
        let var_prefix = if host == target { "HOST" } else { "TARGET" };
        let target_with_underscores = target.replace('-', "_");

        let target_only_var = |name: &str| {
            var(&format!("{name}_{target}"))
                .or_else(|| var(&format!("{name}_{target_with_underscores}")))
                .or_else(|| var(&format!("{var_prefix}_{name}")))
        };
        let target_var = |name: &str| target_only_var(name).or_else(|| var(name));

        Self {
            path: target_var("LEYLINE_BSSL_PATH").map(PathBuf::from),
            include_path: target_var("LEYLINE_BSSL_INCLUDE_PATH").map(PathBuf::from),
            source_path: target_var("LEYLINE_BSSL_SOURCE_PATH").map(PathBuf::from),
            assume_patched: target_var("LEYLINE_BSSL_ASSUME_PATCHED")
                .is_some_and(|v| !v.is_empty()),
            sysroot: target_var("LEYLINE_BSSL_SYSROOT").map(PathBuf::from),
            compiler_external_toolchain: target_var("LEYLINE_BSSL_COMPILER_EXTERNAL_TOOLCHAIN")
                .map(PathBuf::from),
            debug: target_var("DEBUG"),
            opt_level: target_var("OPT_LEVEL"),
            android_ndk_home: target_var("ANDROID_NDK_HOME").map(Into::into),
            cmake_toolchain_file: target_var("CMAKE_TOOLCHAIN_FILE").map(Into::into),
            cpp_runtime_lib: target_var("LEYLINE_BSSL_RUST_CPPLIB"),
            cc: target_only_var("CC"),
            cxx: target_only_var("CXX"),
            docs_rs: var("DOCS_RS").is_some(),
        }
    }
}

fn var(name: &str) -> Option<OsString> {
    println!("cargo:rerun-if-env-changed={name}");

    env::var_os(name)
}
