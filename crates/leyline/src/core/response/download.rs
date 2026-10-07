use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

struct CallerWaits(Arc<AtomicBool>);

impl Default for CallerWaits {
    fn default() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }
}

impl Drop for CallerWaits {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Response {
    pub async fn download_to(self, path: impl AsRef<Path>, limit: Option<u64>) -> Result<u64> {
        let path = path.as_ref().to_path_buf();
        let partial = PartialFile::new(atomic::temp_path(&path)?);
        let written = self.write_file(&partial.path, &path, limit).await?;
        let caller = CallerWaits::default();
        let waiting = Arc::clone(&caller.0);
        tokio::task::spawn_blocking(move || {
            if !waiting.load(Ordering::Acquire) {
                drop(partial);
                return Err(Error::from(std::io::Error::from(
                    std::io::ErrorKind::Interrupted,
                )));
            }
            let temp = partial.disarm();
            atomic::commit(&temp, &path)
        })
        .await
        .map_err(|err| Error::from(std::io::Error::other(err)))??;
        drop(caller);
        Ok(written)
    }

    async fn write_file(self, temp: &Path, target: &Path, limit: Option<u64>) -> Result<u64> {
        let mut file = tokio::fs::File::from_std(atomic::create_temp(
            temp,
            target,
            atomic::FileMode::KeepExisting,
        )?);
        let written = self.copy_decoded_to(&mut file, limit).await?;
        file.flush().await?;
        file.sync_all().await?;
        Ok(written)
    }
}
