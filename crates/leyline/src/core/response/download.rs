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
            drop(std::fs::remove_file(&self.path));
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
        let mut file = tokio::fs::File::from_std(atomic::create_temp(
            temp,
            target,
            atomic::FileMode::KeepExisting,
        )?);
        let written = self.write_to(&mut file).await?;
        file.flush().await?;
        file.sync_all().await?;
        Ok(written)
    }
}
