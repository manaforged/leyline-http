use std::sync::{Arc, Mutex};

use super::{Autosave, SaveJob, SaveTarget, Schedule, WriteOrder};

fn recorder(
    log: &Arc<Mutex<Vec<u32>>>,
    value: u32,
) -> Box<dyn FnOnce() -> crate::core::Result<()> + Send> {
    let log = Arc::clone(log);
    Box::new(move || {
        log.lock().unwrap().push(value);
        Ok(())
    })
}

#[test]
fn an_older_save_that_runs_last_does_not_overwrite_a_newer_one() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut order = WriteOrder::default();
    let older = order.order(recorder(&log, 1));
    let newer = order.order(recorder(&log, 2));
    newer().unwrap();
    older().unwrap();
    assert_eq!(*log.lock().unwrap(), [2]);
}

struct FailsOnce {
    attempts: Arc<Mutex<u32>>,
    saved: Arc<Mutex<bool>>,
}

impl SaveTarget for FailsOnce {
    fn prepare(&mut self) -> Option<SaveJob> {
        let attempts = Arc::clone(&self.attempts);
        Some(SaveJob {
            generation: 1,
            write: Box::new(move || {
                let mut attempts = attempts.lock().unwrap();
                *attempts += 1;
                if *attempts == 1 {
                    return Err(super::stopped("test"));
                }
                Ok(())
            }),
        })
    }

    fn saved(&mut self, _generation: u64) {
        *self.saved.lock().unwrap() = true;
    }
}

#[tokio::test]
async fn a_shutdown_save_that_fails_is_written_again_before_the_task_ends() {
    let saved = Arc::new(Mutex::new(false));
    let attempts = Arc::new(Mutex::new(0));
    let (_changes, watch) = tokio::sync::watch::channel(0);
    let target = FailsOnce {
        attempts: Arc::clone(&attempts),
        saved: Arc::clone(&saved),
    };
    let schedule = Schedule {
        interval: std::time::Duration::from_secs(60),
        periodic: None,
        label: "test",
    };
    let writer = Autosave::spawn(target, watch, schedule);
    writer.shutdown().await.unwrap();
    assert_eq!(*attempts.lock().unwrap(), 2);
    assert!(*saved.lock().unwrap());
}
