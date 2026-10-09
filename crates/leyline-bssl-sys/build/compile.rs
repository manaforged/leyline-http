use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::archive::archive_file;
use crate::config::Config;
use crate::fingerprint;
use crate::prefix::PREFIX;
use crate::source::get_boringssl_source_path;

pub(crate) const SOURCES_JSON: &str = "gen/sources.json";
const LIBRARIES: [(&str, &[&str]); 2] = [("crypto", &["bcm", "crypto"]), ("ssl", &["ssl"])];
const PREBUILT_NASM: &str = "prebuilt";
const BUILD_DIR: &str = "bssl-build";
const BUNDLED_SOURCE: &str = "deps/boringssl";
const NASM_FORMAT: &str = "win64";
const NASM_INCLUDES: [&str; 2] = ["-Igen/", "-Iinclude/"];
const CXX_STANDARD: &str = "c++17";
const UNIX_FLAGS: [&str; 5] = [
    "-fno-exceptions",
    "-fno-rtti",
    "-fno-strict-aliasing",
    "-fno-common",
    "-fvisibility=hidden",
];
const MSVC_FLAGS: [&str; 2] = ["/utf-8", "/Zc:__cplusplus"];
const MSVC_DEFINES: [&str; 4] = [
    "_HAS_EXCEPTIONS=0",
    "WIN32_LEAN_AND_MEAN",
    "NOMINMAX",
    "_CRT_SECURE_NO_WARNINGS",
];
const ELF_ASM_FLAGS: [&str; 1] = ["-Wa,--noexecstack"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Assembly {
    Gas,
    Nasm,
}

impl Assembly {
    fn for_target(config: &Config) -> Self {
        if config.target_os == "windows" && config.target_arch == "x86_64" {
            Self::Nasm
        } else {
            Self::Gas
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Gas => "asm",
            Self::Nasm => "nasm",
        }
    }
}

fn source_lists(src: &Path, groups: &[&str], key: &str) -> io::Result<Vec<PathBuf>> {
    let text = fs::read_to_string(src.join(SOURCES_JSON))?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(io::Error::other)?;
    let mut files = Vec::new();
    for group in groups {
        let listed = json
            .get(group)
            .and_then(|g| g.get(key))
            .and_then(serde_json::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for name in listed {
            let name = name.as_str().ok_or_else(|| {
                io::Error::other(format!(
                    "{SOURCES_JSON}: {group}.{key} has a non-string entry"
                ))
            })?;
            files.push(src.join(name));
        }
    }
    Ok(files)
}

fn nasm_program() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("NASM").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let name = if cfg!(windows) { "nasm.exe" } else { "nasm" };
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

fn object_name(source: &Path) -> io::Result<String> {
    source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| format!("{stem}.obj"))
        .ok_or_else(|| io::Error::other(format!("{} has no file name", source.display())))
}

fn assemble(nasm: &Path, src: &Path, source: &Path, object: &Path) -> io::Result<()> {
    let relative = source.strip_prefix(src).map_err(io::Error::other)?;
    let status = Command::new(nasm)
        .current_dir(src)
        .args(["-f", NASM_FORMAT, "--reproducible"])
        .arg(format!("-DBORINGSSL_PREFIX={PREFIX}"))
        .args(NASM_INCLUDES)
        .arg("-o")
        .arg(object)
        .arg(relative)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "nasm failed on {} with {status}",
            source.display()
        )))
    }
}

fn nasm_objects(config: &Config, src: &Path, groups: &[&str]) -> io::Result<Vec<PathBuf>> {
    let sources = source_lists(src, groups, Assembly::Nasm.key())?;
    let prebuilt = config.manifest_dir.join(PREBUILT_NASM).join(&config.target);
    let Some(nasm) = nasm_program() else {
        if config.env.source_path.is_some() {
            return Err(io::Error::other(
                "LEYLINE_BSSL_SOURCE_PATH needs NASM on PATH or in NASM for this target; \
                 the committed objects match only the bundled BoringSSL",
            ));
        }
        return sources
            .iter()
            .map(|source| {
                let object = prebuilt.join(object_name(source)?);
                if object.is_file() {
                    Ok(object)
                } else {
                    Err(io::Error::other(format!(
                        "{} is missing; install NASM or restore the committed objects",
                        object.display()
                    )))
                }
            })
            .collect();
    };
    let out = config.out_dir.join("nasm");
    fs::create_dir_all(&out)?;
    let mut objects = Vec::new();
    for source in &sources {
        let object = out.join(object_name(source)?);
        assemble(&nasm, src, source, &object)?;
        if let Some(dir) = &config.env.prebuilt_nasm_out {
            fs::create_dir_all(dir)?;
            fs::copy(&object, dir.join(object_name(source)?))?;
        }
        objects.push(object);
    }
    println!("cargo:rerun-if-env-changed=NASM");
    Ok(objects)
}

fn base_build(config: &Config, src: &Path) -> cc::Build {
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std(CXX_STANDARD)
        .cargo_metadata(false)
        .warnings(false)
        .include(src.join("include"))
        .define("BORINGSSL_IMPLEMENTATION", None)
        .define("BORINGSSL_PREFIX", PREFIX);
    if config.target_env == "msvc" {
        for flag in MSVC_FLAGS {
            build.flag(flag);
        }
        for define in MSVC_DEFINES {
            let (name, value) = define
                .split_once('=')
                .map_or((define, None), |(n, v)| (n, Some(v)));
            build.define(name, value);
        }
    } else {
        for flag in UNIX_FLAGS {
            build.flag(flag);
        }
        if config.target_os == "linux" {
            for flag in ELF_ASM_FLAGS {
                build.asm_flag(flag);
            }
        }
    }
    map_build_paths(config, src, &mut build);
    build
}

fn map_build_paths(config: &Config, src: &Path, build: &mut cc::Build) {
    for (from, to) in [
        (src, "/build/boringssl"),
        (config.out_dir.as_path(), "/build"),
    ] {
        if config.target_env == "msvc" {
            build.flag_if_supported(format!("/d1trimfile:{}\\", from.display()));
        } else {
            build.flag_if_supported(format!("-ffile-prefix-map={}={to}", from.display()));
        }
    }
}

fn build_library(
    config: &Config,
    src: &Path,
    name: &str,
    groups: &[&str],
    out: &Path,
) -> io::Result<()> {
    let assembly = Assembly::for_target(config);
    let mut build = base_build(config, src);
    build.out_dir(out);
    build.files(source_lists(src, groups, "srcs")?);
    match assembly {
        Assembly::Gas => {
            build.files(source_lists(src, groups, assembly.key())?);
        }
        Assembly::Nasm => {
            for object in nasm_objects(config, src, groups)? {
                build.object(object);
            }
        }
    }
    build.try_compile(name).map_err(io::Error::other)
}

fn watch_sources(config: &Config, prebuilt: &Path) {
    let source = config
        .env
        .source_path
        .clone()
        .unwrap_or_else(|| config.manifest_dir.join(BUNDLED_SOURCE));
    for path in [source.as_path(), prebuilt] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

pub(crate) fn build_boringssl_or_get_prebuilt(config: &Config) -> io::Result<&'static Path> {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    if let Some(path) = BUILT.get() {
        return Ok(path);
    }
    let path = if let Some(path) = &config.env.path {
        if !path.exists() {
            println!("cargo:warning=built path doesn't exist: {}", path.display());
        }
        path.clone()
    } else {
        let src = get_boringssl_source_path(config);
        let out = config.out_dir.join(BUILD_DIR);
        fs::create_dir_all(&out)?;
        let prebuilt = config.manifest_dir.join(PREBUILT_NASM);
        watch_sources(config, &prebuilt);
        let compiler = base_build(config, src).get_compiler();
        let extra = [config.target.clone(), format!("{:?}", nasm_program())];
        let stamp = fingerprint::inputs(&[src, &prebuilt], &compiler, &extra)?;
        let outputs: Vec<String> = LIBRARIES
            .iter()
            .map(|(name, _)| archive_file(config, name))
            .collect();
        if !fingerprint::is_current(&out, &stamp, &outputs) {
            for (name, groups) in LIBRARIES {
                build_library(config, src, name, groups, &out)?;
            }
            fingerprint::record(&out, &stamp)?;
        }
        out
    };
    Ok(BUILT.get_or_init(|| path))
}
