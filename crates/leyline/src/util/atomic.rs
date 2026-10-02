use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::core::{Error, Kind, Result};

pub(crate) fn write_atomic(path: &Path, bytes: &[u8], mode: FileMode) -> Result<()> {
    let temp = temp_path(path)?;
    let written = write_synced(&temp, path, bytes, mode);
    if let Err(err) = written {
        drop(std::fs::remove_file(&temp));
        return Err(err);
    }
    commit(&temp, path)
}

pub(crate) fn commit(temp: &Path, path: &Path) -> Result<()> {
    if let Err(err) = std::fs::rename(temp, path) {
        drop(std::fs::remove_file(temp));
        return Err(err.into());
    }
    sync_parent(path)
}

fn write_synced(temp: &Path, target: &Path, bytes: &[u8], mode: FileMode) -> Result<()> {
    let mut file = create_temp(temp, target, mode)?;
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

pub(crate) fn temp_path(path: &Path) -> Result<PathBuf> {
    let Some(name) = path.file_name() else {
        return Err(Error::new(Kind::Request).with_message("file path has no file name"));
    };
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(name);
    temp_name.push(format!(".{:016x}.part", rand::random::<u64>()));
    Ok(path.with_file_name(temp_name))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileMode {
    Private,
    KeepExisting,
}

#[cfg(unix)]
pub(crate) fn create_temp(temp: &Path, target: &Path, mode: FileMode) -> Result<std::fs::File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let perms = match mode {
        FileMode::Private => Some(std::fs::Permissions::from_mode(0o600)),
        FileMode::KeepExisting => std::fs::metadata(target)
            .ok()
            .map(|meta| std::fs::Permissions::from_mode(meta.permissions().mode() & 0o777)),
    };
    if let Some(perms) = &perms {
        options.mode(perms.mode());
    }
    let file = options.open(temp)?;
    if let Some(perms) = perms {
        file.set_permissions(perms)?;
    }
    Ok(file)
}

#[cfg(not(unix))]
pub(crate) fn create_temp(temp: &Path, _target: &Path, _mode: FileMode) -> Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    Ok(options.open(temp)?)
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<()> {
    Ok(())
}
