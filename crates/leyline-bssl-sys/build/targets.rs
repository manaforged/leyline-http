use std::ffi::OsString;
use std::io;
use std::path::Path;

use crate::config::Config;

const APPLE_SDKS: [(&str, &str); 8] = [
    ("aarch64-apple-ios", "iphoneos"),
    ("aarch64-apple-ios-sim", "iphonesimulator"),
    ("x86_64-apple-ios", "iphonesimulator"),
    ("aarch64-apple-tvos", "appletvos"),
    ("aarch64-apple-tvos-sim", "appletvsimulator"),
    ("x86_64-apple-tvos", "appletvsimulator"),
    ("aarch64-apple-darwin", "macosx"),
    ("x86_64-apple-darwin", "macosx"),
];

pub(crate) fn get_apple_sdk_name(config: &Config) -> &'static str {
    APPLE_SDKS
        .iter()
        .find(|(target, _)| *target == config.target)
        .map(|(_, sdk)| *sdk)
        .unwrap_or_else(|| panic!("no Apple SDK is known for {}", config.target))
}

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
