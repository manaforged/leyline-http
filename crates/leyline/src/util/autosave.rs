use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};

use crate::core::{Error, Kind, Result};

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
}

struct Guard<T: SaveTarget> {
    target: T,
    changes: watch::Receiver<u64>,
    commands: mpsc::UnboundedReceiver<Command>,
    label: &'static str,
    dirty: bool,
    finished: bool,
}

impl<T: SaveTarget> Guard<T> {
    async fn save(&mut self) -> Result<()> {
        let Some(job) = self.begin() else {
            return Ok(());
        };
        let generation = job.generation;
        let written = tokio::task::spawn_blocking(job.write)
            .await
            .map_err(|_| stopped(self.label))
            .and_then(|out| out);
        self.end(generation, written)
    }

    fn save_now(&mut self) -> Result<()> {
        let Some(job) = self.begin() else {
            return Ok(());
        };
        let generation = job.generation;
        let written = (job.write)();
        self.end(generation, written)
    }

    fn begin(&mut self) -> Option<SaveJob> {
        self.dirty = false;
        self.target.prepare()
    }

    fn end(&mut self, generation: u64, written: Result<()>) -> Result<()> {
        match written {
            Ok(()) => self.target.saved(generation),
            Err(_) => self.dirty = true,
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
    Touch,
    Flush(Reply),
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
        let task = tokio::spawn(run(target, changes, schedule, commands));
        Self {
            control,
            task,
            label,
        }
    }

    pub(crate) fn touch(&self) {
        drop(self.control.send(Command::Touch));
    }

    pub(crate) async fn flush(&self) -> Result<()> {
        let (reply, answer) = oneshot::channel();
        self.control
            .send(Command::Flush(reply))
            .map_err(|_| stopped(self.label))?;
        answer.await.map_err(|_| stopped(self.label))?
    }

    pub(crate) async fn shutdown(self) -> Result<()> {
        let (reply, answer) = oneshot::channel();
        self.control
            .send(Command::Shutdown(reply))
            .map_err(|_| stopped(self.label))?;
        let out = answer.await.map_err(|_| stopped(self.label))?;
        drop(self.task.await);
        out
    }
}

pub(crate) fn stopped(label: &str) -> Error {
    Error::new(Kind::Io).with_message(format!("{label} autosave task stopped"))
}

async fn run<T: SaveTarget>(
    target: T,
    changes: watch::Receiver<u64>,
    schedule: Schedule,
    commands: mpsc::UnboundedReceiver<Command>,
) {
    let mut guard = Guard {
        target,
        changes,
        commands,
        label: schedule.label,
        dirty: false,
        finished: false,
    };
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
                drop(guard.save().await);
            }
            command = guard.commands.recv() => {
                if !handle(command, &mut guard, &mut due, schedule.interval).await {
                    guard.finished = true;
                    return;
                }
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

async fn handle<T: SaveTarget>(
    command: Option<Command>,
    guard: &mut Guard<T>,
    due: &mut Option<Instant>,
    interval: Duration,
) -> bool {
    let reply = match command {
        Some(Command::Touch) => {
            guard.dirty = true;
            due.get_or_insert_with(|| Instant::now() + interval);
            return true;
        }
        Some(Command::Flush(reply)) => Some(reply),
        Some(Command::Shutdown(reply)) => {
            drop(reply.send(guard.save().await));
            return false;
        }
        None => None,
    };
    *due = None;
    let saved = guard.save().await;
    match reply {
        Some(reply) => {
            drop(reply.send(saved));
            true
        }
        None => false,
    }
}
