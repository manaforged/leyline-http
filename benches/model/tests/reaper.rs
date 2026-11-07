use loom::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use loom::sync::{Arc, Mutex};
use loom::thread;

struct Pool {
    entries: Mutex<usize>,
    reaping: AtomicBool,
    reapers: AtomicUsize,
}

impl Pool {
    fn with_reaper(entries: usize) -> Self {
        Self {
            entries: Mutex::new(entries),
            reaping: AtomicBool::new(true),
            reapers: AtomicUsize::new(1),
        }
    }

    fn insert(&self) {
        *self.entries.lock().unwrap() += 1;
        self.watch_idle();
    }

    fn watch_idle(&self) {
        if self.reaping.load(Ordering::Relaxed) {
            return;
        }
        if self.reaping.swap(true, Ordering::AcqRel) {
            return;
        }
        self.reapers.fetch_add(1, Ordering::SeqCst);
    }

    fn evict_all(&self) {
        *self.entries.lock().unwrap() = 0;
    }

    fn retire(&self, recheck: bool) -> bool {
        self.reaping.store(false, Ordering::Release);
        if !recheck {
            return true;
        }
        let refilled = *self.entries.lock().unwrap() > 0;
        !refilled || self.reaping.swap(true, Ordering::AcqRel)
    }

    fn reap_once(&self, recheck: bool) {
        self.evict_all();
        if self.retire(recheck) {
            self.reapers.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn check(&self) {
        let reapers = self.reapers.load(Ordering::SeqCst);
        assert!(reapers <= 1, "{reapers} reapers run at once");
        if *self.entries.lock().unwrap() > 0 {
            assert_eq!(reapers, 1, "entries are pooled with no reaper");
            assert!(
                self.reaping.load(Ordering::SeqCst),
                "a reaper runs unmarked"
            );
        }
    }
}

fn race(recheck: bool, inserters: usize) {
    loom::model(move || {
        let pool = Arc::new(Pool::with_reaper(1));
        let threads: Vec<_> = (0..inserters)
            .map(|_| {
                let pool = Arc::clone(&pool);
                thread::spawn(move || pool.insert())
            })
            .collect();
        pool.reap_once(recheck);
        for thread in threads {
            thread.join().unwrap();
        }
        pool.check();
    });
}

#[test]
fn a_connection_pooled_while_the_reaper_retires_keeps_one_reaper() {
    race(true, 1);
}

#[test]
fn two_connections_pooled_while_the_reaper_retires_start_at_most_one_reaper() {
    race(true, 2);
}

#[test]
#[should_panic(expected = "entries are pooled with no reaper")]
fn retiring_without_the_recheck_strands_a_connection() {
    race(false, 1);
}
