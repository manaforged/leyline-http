use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::core::Result;
use crate::util::autosave::{Autosave, SaveJob, SaveTarget, Schedule};

use super::Jar;

pub struct JarAutosave {
    path: Arc<Path>,
    autosave: Autosave,
}

struct JarTarget {
    jar: Jar,
    path: Arc<Path>,
    saved: Option<u64>,
}

impl Jar {
    pub fn autosave(&self, path: impl Into<PathBuf>, interval: Duration) -> JarAutosave {
        let path: Arc<Path> = Arc::from(path.into());
        let target = JarTarget {
            jar: self.clone(),
            path: Arc::clone(&path),
            saved: None,
        };
        let schedule = Schedule {
            interval,
            periodic: None,
            label: "cookie jar",
        };
        JarAutosave {
            path,
            autosave: Autosave::spawn(target, self.changes(), schedule),
        }
    }
}

impl JarAutosave {
    pub async fn flush(&self) -> Result<()> {
        self.autosave.flush().await
    }

    pub async fn shutdown(self) -> Result<()> {
        self.autosave.shutdown().await
    }
}

impl std::fmt::Debug for JarAutosave {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JarAutosave")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl SaveTarget for JarTarget {
    fn prepare(&mut self) -> Option<SaveJob> {
        let generation = *self.jar.changes.borrow();
        if self.saved == Some(generation) {
            return None;
        }
        let jar = self.jar.clone();
        let path = Arc::clone(&self.path);
        Some(SaveJob {
            generation,
            write: Box::new(move || jar.save_to(&*path)),
        })
    }

    fn saved(&mut self, generation: u64) {
        self.saved = Some(generation);
    }
}
