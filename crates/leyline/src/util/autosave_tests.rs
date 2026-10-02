use std::sync::{Arc, Mutex};

use super::WriteOrder;

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
