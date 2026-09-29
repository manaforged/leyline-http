use fslock::LockFile;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::config::Config;
use crate::process::{git, run_command};

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
        fs_extra::dir::copy(submodule_path, &config.out_dir, &Default::default())
            .inspect_err(|_| {
                let _ = fs::remove_dir_all(&config.out_dir);
            })
            .expect("copying failed. Try running `cargo clean`");

        let src_git_path = src_path.join(".git");
        let _ = fs::remove_file(&src_git_path);
        let _ = fs::remove_dir_all(&src_git_path);

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
    let has_git = src_path.join(".git").exists();

    lock_file.lock()?;

    if !has_git {
        run_command(git(src_path).arg("init"))?;
    }

    let mut patches = fs::read_dir(config.manifest_dir.join("patches"))?
        .map(|entry| entry.map(|e| e.file_name()))
        .collect::<io::Result<Vec<_>>>()?;
    patches.retain(|name| {
        Path::new(name)
            .extension()
            .is_some_and(|ext| ext == "patch")
    });
    patches.sort();
    for patch in patches {
        println!("cargo:rerun-if-changed=patches/{}", patch.to_string_lossy());
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
    let cmd_path = config.manifest_dir.join("patches").join(patch_name);

    let mut args = vec!["apply", "-v", "--whitespace=fix"];

    if config.is_bazel {
        args.push("-p2");
    }

    run_command(git(src_path).args(&args).arg(cmd_path))?;

    Ok(())
}
