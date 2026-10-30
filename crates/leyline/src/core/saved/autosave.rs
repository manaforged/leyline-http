use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

use super::ConnectionState;
use crate::core::Session;
use crate::core::error::Error;
use crate::core::session::SessionInner;
use crate::util::atomic::{FileMode, write_atomic};
use crate::util::autosave::{Autosave as Writer, SaveJob, SaveTarget, Schedule};

struct SessionTarget {
    session: Weak<SessionInner>,
    path: Arc<Path>,
    writing: Option<ConnectionState>,
    saved_state: Option<ConnectionState>,
}

impl Session {
    /// Saves the session to `path` in the background once every `every` interval, and once more on `shutdown`.
    pub fn autosave(&self, path: impl Into<PathBuf>, every: Duration) {
        let target = SessionTarget {
            session: self.downgrade(),
            path: Arc::from(path.into()),
            writing: None,
            saved_state: None,
        };
        let schedule = Schedule {
            interval: every,
            periodic: Some(every),
            label: "session",
        };
        let writer = Writer::spawn(target, self.cookies().changes(), schedule);
        tokio::spawn(writer.until(self.stopped()));
    }
}

impl SaveTarget for SessionTarget {
    fn prepare(&mut self) -> Option<SaveJob> {
        let session = Session::upgrade(&self.session)?;
        let generation = *session.cookies().changes().borrow();
        self.writing = Some(session.connection_state());
        let bytes = serde_json::to_vec_pretty(&session);
        let path = Arc::clone(&self.path);
        Some(SaveJob {
            generation,
            write: Box::new(move || {
                let bytes = bytes.map_err(Error::from_json)?;
                write_atomic(&path, &bytes, FileMode::Private)
            }),
        })
    }

    fn saved(&mut self, _generation: u64) {
        self.saved_state = self.writing.take();
    }

    fn unsaved(&self) -> bool {
        Session::upgrade(&self.session)
            .is_some_and(|session| self.saved_state.as_ref() != Some(&session.connection_state()))
    }

    fn alive(&self) -> bool {
        self.session.strong_count() > 0
    }
}

#[cfg(test)]
mod tests;
