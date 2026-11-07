use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{Autosave, Job, SessionTarget, WriteOrder};
use crate::core::session::Session;

#[tokio::test]
async fn a_dropped_session_is_freed_while_autosave_runs() {
    let dir = std::env::temp_dir().join(format!("leyline-autosave-drop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let session = Session::builder().build().unwrap();
    let weak = session.downgrade();
    session.autosave(dir.join("session.json"), Duration::from_millis(10));
    drop(session);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(weak.strong_count(), 0);
    drop(std::fs::remove_dir_all(&dir));
}

fn recorder(log: &Arc<Mutex<Vec<u32>>>, value: u32) -> Job {
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

static ATTEMPTS: AtomicU32 = AtomicU32::new(0);
static SAVED: AtomicBool = AtomicBool::new(false);

fn fails_once(_: &Path, _: &[u8]) -> crate::error::Result<()> {
    if ATTEMPTS.fetch_add(1, Ordering::SeqCst) == 0 {
        return Err(super::stopped());
    }
    SAVED.store(true, Ordering::SeqCst);
    Ok(())
}

#[tokio::test]
async fn a_shutdown_save_that_fails_is_written_again_before_the_task_ends() {
    let session = Session::builder().build().unwrap();
    let (_changes, watch) = tokio::sync::watch::channel(0);
    let target = SessionTarget {
        session: session.downgrade(),
        path: Arc::from(Path::new("unused.json")),
        write: fails_once,
        writing: None,
        saved_state: None,
    };
    let writer = Autosave::spawn(target, watch, Duration::from_secs(60));
    writer.shutdown().await.unwrap();
    assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 2);
    assert!(SAVED.load(Ordering::SeqCst));
}
