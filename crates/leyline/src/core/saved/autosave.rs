use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};

use super::ConnectionState;
use crate::core::atomic::{FileMode, write_atomic};
use crate::core::session::Session;
use crate::core::session::SessionInner;
use crate::error::{Error, Kind, Result};
use crate::util::lock;

type Job = Box<dyn FnOnce() -> Result<()> + Send>;
type WriteFile = fn(&Path, &[u8]) -> Result<()>;

struct SessionTarget {
    session: Weak<SessionInner>,
    path: Arc<Path>,
    write: WriteFile,
    writing: Option<ConnectionState>,
    saved_state: Option<ConnectionState>,
}

impl Session {
    /// Saves the session to `path` in the background once every `every` interval, and once more on `shutdown`.
    pub fn autosave(&self, path: impl Into<PathBuf>, every: Duration) {
        let target = SessionTarget {
            session: self.downgrade(),
            path: Arc::from(path.into()),
            write: write_private,
            writing: None,
            saved_state: None,
        };
        let writer = Autosave::spawn(target, self.cookies().changes(), every);
        self.track_writer(tokio::spawn(writer.until(self.stopped())));
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    write_atomic(path, bytes, FileMode::Private)
}

impl SessionTarget {
    fn prepare(&mut self) -> Option<Job> {
        let session = Session::upgrade(&self.session)?;
        self.writing = Some(session.connection_state());
        let bytes = serde_json::to_vec_pretty(&session);
        let path = Arc::clone(&self.path);
        let write = self.write;
        Some(Box::new(move || {
            let bytes = bytes.map_err(Error::from_json)?;
            write(&path, &bytes)
        }))
    }

    fn saved(&mut self) {
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

struct Guard {
    target: SessionTarget,
    changes: watch::Receiver<u64>,
    commands: mpsc::UnboundedReceiver<Command>,
    dirty: bool,
    finished: bool,
    order: WriteOrder,
}

#[derive(Default)]
struct WriteOrder {
    sequence: u64,
    written: Arc<Mutex<u64>>,
}

impl WriteOrder {
    fn order(&mut self, write: Job) -> impl FnOnce() -> Result<()> + Send + 'static {
        self.sequence += 1;
        let sequence = self.sequence;
        let written = Arc::clone(&self.written);
        move || {
            let mut last = lock(&written);
            if *last >= sequence {
                return Ok(());
            }
            write()?;
            *last = sequence;
            Ok(())
        }
    }
}

impl Guard {
    async fn save(&mut self) -> Result<()> {
        let Some(write) = self.begin() else {
            return Ok(());
        };
        let written = tokio::task::spawn_blocking(self.order.order(write))
            .await
            .map_err(|e| stopped().with_source(e.to_string()))
            .and_then(|out| out);
        self.end(written)
    }

    fn save_now(&mut self) -> Result<()> {
        let Some(write) = self.begin() else {
            return Ok(());
        };
        let written = self.order.order(write)();
        self.end(written)
    }

    fn begin(&mut self) -> Option<Job> {
        self.dirty = false;
        self.target.prepare()
    }

    fn end(&mut self, written: Result<()>) -> Result<()> {
        match &written {
            Ok(()) => self.target.saved(),
            Err(error) => {
                self.dirty = true;
                tracing::warn!(target: "leyline::autosave", label = LABEL, %error, "autosave write failed");
            }
        }
        written
    }

    fn pending(&mut self) -> bool {
        self.dirty
            || self.changes.has_changed().unwrap_or(false)
            || !self.commands.is_empty()
            || self.target.unsaved()
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if !self.finished && self.pending() {
            drop(self.save_now());
        }
    }
}

const LABEL: &str = "session";

type Reply = oneshot::Sender<Result<()>>;

enum Command {
    Shutdown(Reply),
}

struct Autosave {
    control: mpsc::UnboundedSender<Command>,
    task: JoinHandle<()>,
}

impl Autosave {
    fn spawn(target: SessionTarget, changes: watch::Receiver<u64>, every: Duration) -> Self {
        let (control, commands) = mpsc::unbounded_channel();
        let guard = Guard {
            target,
            changes,
            commands,
            dirty: false,
            finished: false,
            order: WriteOrder::default(),
        };
        let task = tokio::spawn(run(guard, every));
        Self { control, task }
    }

    async fn shutdown(self) -> Result<()> {
        let (reply, answer) = oneshot::channel();
        self.control
            .send(Command::Shutdown(reply))
            .map_err(|e| stopped().with_source(e.to_string()))?;
        let out = answer
            .await
            .map_err(|e| stopped().with_source(e.to_string()))?;
        drop(self.task.await);
        out
    }

    async fn until(mut self, stop: impl std::future::Future<Output = ()>) {
        tokio::select! {
            () = stop => drop(self.shutdown().await),
            _ = &mut self.task => {}
        }
    }
}

fn stopped() -> Error {
    Error::new(Kind::Io).with_message(format!("{LABEL} autosave task stopped"))
}

async fn run(mut guard: Guard, every: Duration) {
    let mut watching = true;
    let mut due: Option<Instant> = None;
    let mut ticker = tokio::time::interval_at(Instant::now() + every, every);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = guard.changes.changed(), if watching => {
                watching = changed.is_ok();
                if watching {
                    guard.dirty = true;
                    due.get_or_insert_with(|| Instant::now() + every);
                }
            }
            () = deadline(due) => {
                due = None;
                drop(guard.save().await);
            }
            _ = ticker.tick() => {
                if !guard.target.alive() {
                    guard.finished = true;
                    return;
                }
                if guard.pending() {
                    drop(guard.save().await);
                }
            }
            command = guard.commands.recv() => {
                handle(command, &mut guard).await;
                guard.finished = true;
                return;
            }
        }
    }
}

async fn deadline(due: Option<Instant>) {
    match due {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

async fn handle(command: Option<Command>, guard: &mut Guard) {
    let saved = match guard.save().await {
        Err(_) if guard.dirty => guard.save_now(),
        saved => saved,
    };
    if let Some(Command::Shutdown(reply)) = command {
        drop(reply.send(saved));
    }
}

#[cfg(test)]
mod tests;
