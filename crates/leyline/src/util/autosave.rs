use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};

use crate::core::{Cause, Error, Result};
use crate::util::lock;

pub(crate) struct SaveJob {
    pub(crate) generation: u64,
    pub(crate) write: Box<dyn FnOnce() -> Result<()> + Send>,
}

pub(crate) trait SaveTarget: Send + 'static {
    fn prepare(&mut self) -> Option<SaveJob>;
    fn saved(&mut self, generation: u64);
    fn unsaved(&self) -> bool {
        false
    }
    fn alive(&self) -> bool {
        true
    }
}

struct Guard<T: SaveTarget> {
    target: T,
    changes: watch::Receiver<u64>,
    commands: mpsc::UnboundedReceiver<Command>,
    label: &'static str,
    dirty: bool,
    finished: bool,
    order: WriteOrder,
}

#[derive(Default)]
pub(crate) struct WriteOrder {
    sequence: u64,
    written: Arc<Mutex<u64>>,
}

impl WriteOrder {
    pub(crate) fn order(
        &mut self,
        write: Box<dyn FnOnce() -> Result<()> + Send>,
    ) -> impl FnOnce() -> Result<()> + Send + 'static {
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

impl<T: SaveTarget> Guard<T> {
    async fn save(&mut self) -> Result<()> {
        let Some(job) = self.begin() else {
            return Ok(());
        };
        let generation = job.generation;
        let written = tokio::task::spawn_blocking(self.order.order(job.write))
            .await
            .map_err(|e| stopped(self.label).with_source(e.to_string()))
            .and_then(|out| out);
        self.end(generation, written)
    }

    fn save_now(&mut self) -> Result<()> {
        let Some(job) = self.begin() else {
            return Ok(());
        };
        let generation = job.generation;
        let written = self.order.order(job.write)();
        self.end(generation, written)
    }

    fn begin(&mut self) -> Option<SaveJob> {
        self.dirty = false;
        self.target.prepare()
    }

    fn end(&mut self, generation: u64, written: Result<()>) -> Result<()> {
        match &written {
            Ok(()) => self.target.saved(generation),
            Err(error) => {
                self.dirty = true;
                tracing::warn!(target: "leyline::autosave", label = self.label, %error, "autosave write failed");
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

impl<T: SaveTarget> Drop for Guard<T> {
    fn drop(&mut self) {
        if !self.finished && self.pending() {
            drop(self.save_now());
        }
    }
}

type Reply = oneshot::Sender<Result<()>>;

enum Command {
    Shutdown(Reply),
}

pub(crate) struct Autosave {
    control: mpsc::UnboundedSender<Command>,
    task: JoinHandle<()>,
    label: &'static str,
}

pub(crate) struct Schedule {
    pub(crate) interval: Duration,
    pub(crate) periodic: Option<Duration>,
    pub(crate) label: &'static str,
}

impl Autosave {
    pub(crate) fn spawn<T: SaveTarget>(
        target: T,
        changes: watch::Receiver<u64>,
        schedule: Schedule,
    ) -> Self {
        let (control, commands) = mpsc::unbounded_channel();
        let label = schedule.label;
        let guard = Guard {
            target,
            changes,
            commands,
            label,
            dirty: false,
            finished: false,
            order: WriteOrder::default(),
        };
        let task = tokio::spawn(run(guard, schedule));
        Self {
            control,
            task,
            label,
        }
    }

    pub(crate) async fn shutdown(self) -> Result<()> {
        let (reply, answer) = oneshot::channel();
        self.control
            .send(Command::Shutdown(reply))
            .map_err(|e| stopped(self.label).with_source(e.to_string()))?;
        let out = answer
            .await
            .map_err(|e| stopped(self.label).with_source(e.to_string()))?;
        drop(self.task.await);
        out
    }

    pub(crate) async fn until(mut self, stop: impl std::future::Future<Output = ()>) {
        tokio::select! {
            () = stop => drop(self.shutdown().await),
            _ = &mut self.task => {}
        }
    }
}

pub(crate) fn stopped(label: &str) -> Error {
    Error::new(Cause::Io).with_message(format!("{label} autosave task stopped"))
}

async fn run<T: SaveTarget>(mut guard: Guard<T>, schedule: Schedule) {
    let mut watching = true;
    let mut due: Option<Instant> = None;
    let mut ticker = schedule.periodic.map(|period| {
        let mut ticker = tokio::time::interval_at(Instant::now() + period, period);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        ticker
    });
    loop {
        tokio::select! {
            changed = guard.changes.changed(), if watching => {
                watching = changed.is_ok();
                if watching {
                    guard.dirty = true;
                    due.get_or_insert_with(|| Instant::now() + schedule.interval);
                }
            }
            () = deadline(due) => {
                due = None;
                drop(guard.save().await);
            }
            () = tick(ticker.as_mut()) => {
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

async fn tick(ticker: Option<&mut tokio::time::Interval>) {
    match ticker {
        Some(ticker) => drop(ticker.tick().await),
        None => std::future::pending().await,
    }
}

async fn handle<T: SaveTarget>(command: Option<Command>, guard: &mut Guard<T>) {
    let saved = match guard.save().await {
        Err(_) if guard.dirty => guard.save_now(),
        saved => saved,
    };
    if let Some(Command::Shutdown(reply)) = command {
        drop(reply.send(saved));
    }
}

#[cfg(test)]
#[path = "autosave_tests.rs"]
mod tests;
