use std::future::Future;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};

use crate::core::{Error, Kind, Result};

pub(crate) trait SaveTarget: Send + 'static {
    fn save(&mut self) -> impl Future<Output = Result<()>> + Send;
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
    pub(crate) debounce: Duration,
    pub(crate) interval: Option<Duration>,
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
    mut target: T,
    mut changes: watch::Receiver<u64>,
    schedule: Schedule,
    mut commands: mpsc::UnboundedReceiver<Command>,
) {
    let mut watching = true;
    let mut due: Option<Instant> = None;
    let mut ticker = schedule.interval.map(|period| {
        let mut ticker = tokio::time::interval_at(Instant::now() + period, period);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        ticker
    });
    loop {
        tokio::select! {
            changed = changes.changed(), if watching => {
                watching = changed.is_ok();
                if watching {
                    due.get_or_insert_with(|| Instant::now() + schedule.debounce);
                }
            }
            () = deadline(due) => {
                due = None;
                drop(target.save().await);
            }
            () = tick(ticker.as_mut()) => {
                drop(target.save().await);
            }
            command = commands.recv() => {
                if !handle(command, &mut target, &mut due, schedule.debounce).await {
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
    target: &mut T,
    due: &mut Option<Instant>,
    debounce: Duration,
) -> bool {
    let reply = match command {
        Some(Command::Touch) => {
            due.get_or_insert_with(|| Instant::now() + debounce);
            return true;
        }
        Some(Command::Flush(reply)) => Some(reply),
        Some(Command::Shutdown(reply)) => {
            drop(reply.send(target.save().await));
            return false;
        }
        None => None,
    };
    *due = None;
    let saved = target.save().await;
    match reply {
        Some(reply) => {
            drop(reply.send(saved));
            true
        }
        None => false,
    }
}
