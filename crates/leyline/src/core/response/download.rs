use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;

use super::Response;
use crate::core::atomic;
use crate::error::Result;

struct PartialFile {
    path: PathBuf,
    armed: bool,
}

impl PartialFile {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(mut self) -> PathBuf {
        self.armed = false;
        std::mem::take(&mut self.path)
    }
}

impl Drop for PartialFile {
    fn drop(&mut self) {
        if self.armed {
            let path = std::mem::take(&mut self.path);
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                drop(handle.spawn_blocking(move || drop(std::fs::remove_file(&path))));
            } else {
                drop(std::fs::remove_file(&path));
            }
        }
    }
}

impl Response {
    /// Writes the body to a temporary file, renames it to `path`, and returns the number of bytes written.
    pub async fn download_to(self, path: impl AsRef<Path>) -> Result<u64> {
        let path = path.as_ref().to_path_buf();
        let partial = PartialFile::new(atomic::temp_path(&path)?);
        let written = self.write_file(&partial.path, &path).await?;
        atomic::blocking(move |waiting| {
            waiting.check()?;
            let temp = partial.disarm();
            atomic::commit(&temp, &path)
        })
        .await?;
        Ok(written)
    }

    async fn write_file(self, temp: &Path, target: &Path) -> Result<u64> {
        let temp = temp.to_path_buf();
        let target = target.to_path_buf();
        let std_file = atomic::blocking(move |waiting| {
            waiting.check()?;
            atomic::create_temp(&temp, &target, atomic::FileMode::KeepExisting)
        })
        .await?;
        let mut file = tokio::fs::File::from_std(std_file);
        let written = self.write_to(&mut file).await?;
        file.flush().await?;
        file.sync_all().await?;
        Ok(written)
    }
}
