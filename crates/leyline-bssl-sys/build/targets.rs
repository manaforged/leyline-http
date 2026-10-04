#[cfg(feature = "bindgen")]
use std::ffi::OsString;
#[cfg(feature = "bindgen")]
use std::io;
#[cfg(feature = "bindgen")]
use std::path::Path;

use crate::config::Config;

pub(crate) fn should_use_cmake_cross_compilation(config: &Config) -> bool {
    if config.host == config.target {
        return false;
    }
    match config.target_os.as_str() {
        "macos" | "ios" | "tvos" => !config.host.ends_with("-darwin"),
        _ => true,
    }
}

const CMAKE_PARAMS_ANDROID_NDK: &[(&str, &[(&str, &str)])] = &[
    ("aarch64", &[("ANDROID_ABI", "arm64-v8a")]),
    ("arm", &[("ANDROID_ABI", "armeabi-v7a")]),
    ("x86", &[("ANDROID_ABI", "x86")]),
    ("x86_64", &[("ANDROID_ABI", "x86_64")]),
];

pub(crate) fn cmake_params_android(config: &Config) -> &'static [(&'static str, &'static str)] {
    for (android_arch, params) in CMAKE_PARAMS_ANDROID_NDK {
        if *android_arch == config.target_arch {
            return params;
        }
    }
    &[]
}

const CMAKE_PARAMS_APPLE: &[(&str, &[(&str, &str)])] = &[
    (
        "aarch64-apple-ios",
        &[
            ("CMAKE_OSX_ARCHITECTURES", "arm64"),
            ("CMAKE_OSX_SYSROOT", "iphoneos"),
            ("CMAKE_MACOSX_BUNDLE", "OFF"),
        ],
    ),
    (
        "aarch64-apple-ios-sim",
        &[
            ("CMAKE_OSX_ARCHITECTURES", "arm64"),
            ("CMAKE_OSX_SYSROOT", "iphonesimulator"),
            ("CMAKE_MACOSX_BUNDLE", "OFF"),
        ],
    ),
    (
        "x86_64-apple-ios",
        &[
            ("CMAKE_OSX_ARCHITECTURES", "x86_64"),
            ("CMAKE_OSX_SYSROOT", "iphonesimulator"),
            ("CMAKE_MACOSX_BUNDLE", "OFF"),
        ],
    ),
    (
        "aarch64-apple-tvos",
        &[
            ("CMAKE_OSX_ARCHITECTURES", "arm64"),
            ("CMAKE_OSX_SYSROOT", "appletvos"),
            ("CMAKE_MACOSX_BUNDLE", "OFF"),
        ],
    ),
    (
        "aarch64-apple-tvos-sim",
        &[
            ("CMAKE_OSX_ARCHITECTURES", "arm64"),
            ("CMAKE_OSX_SYSROOT", "appletvsimulator"),
            ("CMAKE_MACOSX_BUNDLE", "OFF"),
        ],
    ),
    (
        "x86_64-apple-tvos",
        &[
            ("CMAKE_OSX_ARCHITECTURES", "x86_64"),
            ("CMAKE_OSX_SYSROOT", "appletvsimulator"),
            ("CMAKE_MACOSX_BUNDLE", "OFF"),
        ],
    ),
    (
        "aarch64-apple-darwin",
        &[
            ("CMAKE_OSX_ARCHITECTURES", "arm64"),
            ("CMAKE_OSX_SYSROOT", "macosx"),
        ],
    ),
    (
        "x86_64-apple-darwin",
        &[
            ("CMAKE_OSX_ARCHITECTURES", "x86_64"),
            ("CMAKE_OSX_SYSROOT", "macosx"),
        ],
    ),
];

pub(crate) fn cmake_params_apple(config: &Config) -> &'static [(&'static str, &'static str)] {
    for (next_target, params) in CMAKE_PARAMS_APPLE {
        if *next_target == config.target {
            return params;
        }
    }
    &[]
}

#[cfg(feature = "bindgen")]
pub(crate) fn get_apple_sdk_name(config: &Config) -> &'static str {
    for (name, value) in cmake_params_apple(config) {
        if *name == "CMAKE_OSX_SYSROOT" {
            return value;
        }
    }

    panic!(
        "cannot find SDK for {} in CMAKE_PARAMS_APPLE",
        config.target
    );
}

#[cfg(feature = "bindgen")]
pub(crate) fn pick_best_android_ndk_toolchain(toolchains_dir: &Path) -> io::Result<OsString> {
    let toolchains = std::fs::read_dir(toolchains_dir)?.collect::<Result<Vec<_>, _>>()?;
    for known_toolchain in ["linux-x86_64", "darwin-x86_64", "windows-x86_64"] {
        if let Some(toolchain) = toolchains
            .iter()
            .find(|entry| entry.file_name() == known_toolchain)
        {
            return Ok(toolchain.file_name());
        }
    }
    if let Some(toolchain) = toolchains
        .into_iter()
        .find(|entry| entry.file_type().map(|ty| ty.is_dir()).unwrap_or(false))
    {
        return Ok(toolchain.file_name());
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no subdirectories at given path",
    ))
}
