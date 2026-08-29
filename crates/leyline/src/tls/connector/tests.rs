use super::lock_unpoisoned;
use std::sync::{Arc, Mutex};

#[test]
fn lock_unpoisoned_recovers_after_panic() {
    let cache = Arc::new(Mutex::new(vec![1u8]));
    let poisoner = cache.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoner.lock().unwrap();
        panic!("poison the mutex");
    })
    .join();
    assert!(cache.is_poisoned(), "test setup failed to poison the mutex");

    // Both the read and write paths must keep working.
    lock_unpoisoned(&cache).push(2);
    assert_eq!(*lock_unpoisoned(&cache), vec![1, 2]);
}
