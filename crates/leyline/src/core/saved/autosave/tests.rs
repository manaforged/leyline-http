use std::time::Duration;

use crate::core::Session;

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
