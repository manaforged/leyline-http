use fslock::LockFile;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::config::Config;
use crate::process::{git, run_command};

const PATCHES_DIR: &str = "patches";
const PATCH_EXTENSION: &str = "patch";
const GIT_DIR: &str = ".git";
const PATCH_TARGET_MARKERS: [&str; 2] = ["--- a/", "+++ b/"];

pub(crate) fn get_boringssl_source_path(config: &Config) -> &Path {
    static SOURCE_PATH: OnceLock<PathBuf> = OnceLock::new();

    SOURCE_PATH.get_or_init(|| {
        if let Some(src_path) = &config.env.source_path {
            if !src_path.exists() {
                println!(
                    "cargo:warning=boringssl source path doesn't exist: {}",
                    src_path.display()
                );
            }
            return src_path.into();
        }

        let submodule_dir = "boringssl";

        let src_path = config.out_dir.join(submodule_dir);

        let submodule_path = config.manifest_dir.join("deps").join(submodule_dir);

        if !submodule_path.join("CMakeLists.txt").exists() {
            println!("cargo:warning=fetching boringssl git submodule");

            run_command(
                git(&config.manifest_dir)
                    .args(["submodule", "update", "--init", "--recursive"])
                    .arg(&submodule_path),
            )
            .expect("git submodule update");
        }

        let _ = fs::remove_dir_all(&src_path);
        patched_paths(config)
            .and_then(|patched| link_tree(&submodule_path, &src_path, Path::new(""), &patched))
            .inspect_err(|_| {
                let _ = fs::remove_dir_all(&config.out_dir);
            })
            .expect("copying failed. Try running `cargo clean`");

        src_path
    })
}

pub(crate) fn ensure_patches_applied(config: &Config) -> io::Result<()> {
    if config.env.assume_patched || config.env.path.is_some() {
        println!(
            "cargo:warning=skipping git patches application, provided \
            native BoringSSL is expected to have the patches included"
        );
        return Ok(());
    }

    let mut lock_file = LockFile::open(&config.out_dir.join(".patch_lock"))?;
    let src_path = get_boringssl_source_path(config);
    let has_git = src_path.join(GIT_DIR).exists();

    lock_file.lock()?;

    if !has_git {
        run_command(git(src_path).arg("init"))?;
    }

    println!("cargo:rerun-if-changed={PATCHES_DIR}");
    let patches = patch_names(config)?;
    for patch in patches {
        println!(
            "cargo:rerun-if-changed={PATCHES_DIR}/{}",
            patch.to_string_lossy()
        );
        apply_patch(config, &patch.to_string_lossy())?;
    }

    Ok(())
}

fn apply_patch(config: &Config, patch_name: &str) -> io::Result<()> {
    let src_path = get_boringssl_source_path(config);
    #[cfg(not(windows))]
    let cmd_path = config
        .manifest_dir
        .join("patches")
        .join(patch_name)
        .canonicalize()?;

    #[cfg(windows)]
    let cmd_path = config.manifest_dir.join(PATCHES_DIR).join(patch_name);

    let mut args = vec!["apply", "-v", "--whitespace=fix"];

    if config.is_bazel {
        args.push("-p2");
    }

    run_command(git(src_path).args(&args).arg(cmd_path))?;

    Ok(())
}

fn patch_names(config: &Config) -> io::Result<Vec<OsString>> {
    let mut patches = fs::read_dir(config.manifest_dir.join(PATCHES_DIR))?
        .map(|entry| entry.map(|e| e.file_name()))
        .collect::<io::Result<Vec<_>>>()?;
    patches.retain(|name| {
        Path::new(name)
            .extension()
            .is_some_and(|ext| ext == PATCH_EXTENSION)
    });
    patches.sort();
    Ok(patches)
}

fn patched_paths(config: &Config) -> io::Result<HashSet<PathBuf>> {
    let mut paths = HashSet::new();
    for patch in patch_names(config)? {
        let text = fs::read_to_string(config.manifest_dir.join(PATCHES_DIR).join(patch))?;
        paths.extend(text.lines().filter_map(|line| {
            PATCH_TARGET_MARKERS
                .iter()
                .find_map(|marker| line.strip_prefix(marker))
                .map(|path| PathBuf::from(path.trim_end()))
        }));
    }
    Ok(paths)
}

fn link_tree(from: &Path, to: &Path, rel: &Path, copied: &HashSet<PathBuf>) -> io::Result<()> {
    fs::create_dir_all(to.join(rel))?;
    for entry in fs::read_dir(from.join(rel))? {
        let entry = entry?;
        let name = entry.file_name();
        if rel.as_os_str().is_empty() && name == GIT_DIR {
            continue;
        }
        let child = rel.join(&name);
        if entry.file_type()?.is_dir() {
            link_tree(from, to, &child, copied)?;
        } else {
            place_file(&entry.path(), &to.join(&child), copied.contains(&child))?;
        }
    }
    Ok(())
}

fn place_file(from: &Path, to: &Path, copy: bool) -> io::Result<()> {
    if !copy && fs::hard_link(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to).map(drop)
}
