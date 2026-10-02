use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;

use super::Response;
use crate::core::error::{Error, Result};
use crate::util::atomic;

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
    pub async fn download_to(self, path: impl AsRef<Path>, limit: Option<u64>) -> Result<u64> {
        let path = path.as_ref().to_path_buf();
        let partial = PartialFile::new(atomic::temp_path(&path)?);
        let written = self.write_file(&partial.path, limit).await?;
        let temp = partial.disarm();
        tokio::task::spawn_blocking(move || atomic::commit(&temp, &path))
            .await
            .map_err(|err| Error::from(std::io::Error::other(err)))??;
        Ok(written)
    }

    async fn write_file(self, temp: &Path, limit: Option<u64>) -> Result<u64> {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temp)
            .await?;
        let written = self.copy_decoded_to(&mut file, limit).await?;
        file.flush().await?;
        file.sync_all().await?;
        Ok(written)
    }
}
