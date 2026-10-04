use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::config::Config;
use crate::prefix::PREFIX;
use crate::source::get_boringssl_source_path;
use crate::targets::{
    cmake_params_android, cmake_params_apple, should_use_cmake_cross_compilation,
};

const NINJA_GENERATOR: &str = "Ninja";
const NINJA_PROGRAM: &str = if cfg!(windows) { "ninja.exe" } else { "ninja" };

fn ninja_on_path() -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| dir.join(NINJA_PROGRAM).is_file())
    })
}

fn select_generator(config: &Config, cmake: &mut cmake::Config) {
    if config.env.cmake_generator.is_none() && ninja_on_path() {
        cmake.generator(NINJA_GENERATOR);
    }
}

fn map_build_paths(config: &Config, cmake: &mut cmake::Config) {
    let source = get_boringssl_source_path(config);
    let mut maps = vec![(config.out_dir.as_path(), "/build")];
    if !source.starts_with(&config.out_dir) {
        maps.push((source, "/build/boringssl"));
    }
    for (from, to) in maps {
        let flag = if config.target_env == "msvc" {
            format!("\"/d1trimfile:{}\\\\\"", from.display())
        } else {
            format!("\"-ffile-prefix-map={}={to}\"", from.display())
        };
        cmake.cflag(&flag).cxxflag(&flag);
    }
    if config.target_os == "macos" {
        let min = std::env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "11.0".into());
        cmake.define("CMAKE_OSX_DEPLOYMENT_TARGET", min);
    }
}

fn linux_cross_compiler(config: &Config, cpp: bool) -> Option<OsString> {
    if config.target_os != "linux" {
        return None;
    }
    cc::Build::new()
        .cargo_metadata(false)
        .cpp(cpp)
        .target(&config.target)
        .host(&config.host)
        .try_get_compiler()
        .ok()
        .map(|tool| tool.path().as_os_str().to_owned())
}

fn configure_msvc_runtime(config: &Config, boringssl_cmake: &mut cmake::Config) {
    if config.target_os == "windows" {
        if config.target_features.iter().any(|f| f == "crt-static") {
            boringssl_cmake.define("CMAKE_MSVC_RUNTIME_LIBRARY", "MultiThreaded");
        } else {
            boringssl_cmake.define("CMAKE_MSVC_RUNTIME_LIBRARY", "MultiThreadedDLL");
        }
    }
}

fn configure_cross_compile(config: &Config, boringssl_cmake: &mut cmake::Config) {
    if should_use_cmake_cross_compilation(config) {
        boringssl_cmake
            .define("CMAKE_CROSSCOMPILING", "true")
            .define("CMAKE_C_COMPILER_TARGET", &config.target)
            .define("CMAKE_CXX_COMPILER_TARGET", &config.target)
            .define("CMAKE_ASM_COMPILER_TARGET", &config.target);
    }

    let detect = |cpp: bool| linux_cross_compiler(config, cpp);
    if let Some(cc) = config.env.cc.clone().or_else(|| detect(false)) {
        boringssl_cmake.define("CMAKE_C_COMPILER", cc);
    }
    if let Some(cxx) = config.env.cxx.clone().or_else(|| detect(true)) {
        boringssl_cmake.define("CMAKE_CXX_COMPILER", cxx);
    }

    if let Some(sysroot) = &config.env.sysroot {
        boringssl_cmake.define("CMAKE_SYSROOT", sysroot);
    }

    if let Some(toolchain) = &config.env.compiler_external_toolchain {
        boringssl_cmake
            .define("CMAKE_C_COMPILER_EXTERNAL_TOOLCHAIN", toolchain)
            .define("CMAKE_CXX_COMPILER_EXTERNAL_TOOLCHAIN", toolchain)
            .define("CMAKE_ASM_COMPILER_EXTERNAL_TOOLCHAIN", toolchain);
    }
}

fn configure_android_ndk(config: &Config, boringssl_cmake: &mut cmake::Config) {
    let android_ndk_home = config
        .env
        .android_ndk_home
        .as_ref()
        .expect("Please set ANDROID_NDK_HOME for Android build");
    for (name, value) in cmake_params_android(config) {
        eprintln!("android arch={} add {}={}", config.target_arch, name, value);
        boringssl_cmake.define(name, value);
    }
    let toolchain_file = android_ndk_home.join("build/cmake/android.toolchain.cmake");
    let toolchain_file = toolchain_file.to_str().unwrap();
    eprintln!("android toolchain={toolchain_file}");
    boringssl_cmake.define("CMAKE_TOOLCHAIN_FILE", toolchain_file);

    boringssl_cmake.define("CMAKE_SYSTEM_VERSION", "21");
    boringssl_cmake.define("CMAKE_ANDROID_STL_TYPE", "c++_shared");
}

fn configure_apple_sdk(config: &Config, boringssl_cmake: &mut cmake::Config) {
    match &*config.target_os {
        "macos" => {
            for (name, value) in cmake_params_apple(config) {
                eprintln!("macos arch={} add {}={}", config.target_arch, name, value);
                boringssl_cmake.define(name, value);
            }
        }

        "ios" => {
            for (name, value) in cmake_params_apple(config) {
                eprintln!("ios arch={} add {}={}", config.target_arch, name, value);
                boringssl_cmake.define(name, value);
            }

            let bitcode_cflag = "-fembed-bitcode";

            let target_cflag = if config.target_arch == "x86_64" {
                "-target x86_64-apple-ios-simulator"
            } else {
                ""
            };

            let cflag = format!("{bitcode_cflag} {target_cflag}");
            boringssl_cmake.define("CMAKE_ASM_FLAGS", &cflag);
            boringssl_cmake.cflag(&cflag);
        }

        "tvos" => {
            for (name, value) in cmake_params_apple(config) {
                eprintln!("tvos arch={} add {}={}", config.target_arch, name, value);
                boringssl_cmake.define(name, value);
            }
        }

        _ => {}
    }
}

fn configure_linux_toolchain(
    config: &Config,
    src_path: &Path,
    boringssl_cmake: &mut cmake::Config,
) {
    match &*config.target_arch {
        "x86" => {
            boringssl_cmake.define(
                "CMAKE_TOOLCHAIN_FILE",
                config
                    .manifest_dir
                    .join(src_path)
                    .join("util/32-bit-toolchain.cmake")
                    .as_os_str(),
            );
        }
        "aarch64" => {
            boringssl_cmake.define(
                "CMAKE_TOOLCHAIN_FILE",
                config
                    .manifest_dir
                    .join("cmake/aarch64-linux.cmake")
                    .as_os_str(),
            );
        }
        "arm" => {
            boringssl_cmake.define(
                "CMAKE_TOOLCHAIN_FILE",
                config
                    .manifest_dir
                    .join("cmake/armv7-linux.cmake")
                    .as_os_str(),
            );
        }
        "x86_64" => {}
        _ => {
            println!(
                "cargo:warning=no toolchain file configured for {}",
                config.target
            );
        }
    }
}

fn get_boringssl_cmake_config(config: &Config) -> cmake::Config {
    let src_path = get_boringssl_source_path(config);
    let mut boringssl_cmake = cmake::Config::new(src_path);

    if config.env.cmake_toolchain_file.is_some() {
        return boringssl_cmake;
    }

    configure_msvc_runtime(config, &mut boringssl_cmake);

    if config.host == config.target {
        return boringssl_cmake;
    }

    configure_cross_compile(config, &mut boringssl_cmake);

    match &*config.target_os {
        "android" => configure_android_ndk(config, &mut boringssl_cmake),
        "macos" | "ios" | "tvos" => configure_apple_sdk(config, &mut boringssl_cmake),
        "windows" if config.host.contains("windows") => {
            boringssl_cmake.define("OPENSSL_NO_ASM", "YES");
        }
        "linux" => configure_linux_toolchain(config, src_path, &mut boringssl_cmake),
        _ => {}
    }

    boringssl_cmake
}

pub(crate) fn build_boringssl_or_get_prebuilt(config: &Config) -> &Path {
    static BUILD_SOURCE_PATH: OnceLock<PathBuf> = OnceLock::new();

    BUILD_SOURCE_PATH.get_or_init(|| {
        if let Some(path) = &config.env.path {
            if !path.exists() {
                println!("cargo:warning=built path doesn't exist: {}", path.display());
            }
            return path.into();
        }

        let mut cfg = get_boringssl_cmake_config(config);

        let num_jobs = std::env::var("NUM_JOBS").ok().or_else(|| {
            std::thread::available_parallelism()
                .ok()
                .map(|t| t.to_string())
        });
        if let Some(num_jobs) = num_jobs {
            cfg.env("CMAKE_BUILD_PARALLEL_LEVEL", num_jobs);
        }

        cfg.define("BORINGSSL_PREFIX", PREFIX);
        map_build_paths(config, &mut cfg);
        select_generator(config, &mut cfg);

        cfg.build_target("ssl").build();
        let path = cfg.build_target("crypto").build();
        let build_dir = path.join("build");
        if build_dir.exists() {
            build_dir
        } else {
            path
        }
    })
}
