use loom::sync::atomic::{AtomicUsize, Ordering};
use loom::sync::{Arc, Condvar, Mutex};
use loom::thread;

const CAPACITY: usize = 2;
const WAITING_SENDERS: usize = 2;

#[derive(Default)]
struct Waiters {
    permit: bool,
    waiting: usize,
    granted: usize,
}

struct Notify {
    state: Mutex<Waiters>,
    wake: Condvar,
}

impl Notify {
    fn new() -> Self {
        Self { state: Mutex::new(Waiters::default()), wake: Condvar::new() }
    }

    fn notify(&self) {
        let mut state = self.state.lock().unwrap();
        if state.waiting > state.granted {
            state.granted += 1;
            self.wake.notify_one();
        } else {
            state.permit = true;
        }
    }

    fn wait(&self) {
        let mut state = self.state.lock().unwrap();
        if std::mem::take(&mut state.permit) {
            return;
        }
        state.waiting += 1;
        while state.granted == 0 {
            state = self.wake.wait(state).unwrap();
        }
        state.granted -= 1;
        state.waiting -= 1;
    }
}

#[derive(Clone, Copy, PartialEq)]
enum WakeRule {
    OnlyAtCapacity,
    PassToNextSender,
}

struct Queue {
    rule: WakeRule,
    queued: AtomicUsize,
    items: AtomicUsize,
    space: Notify,
    arrived: Notify,
}

impl Queue {
    fn new(rule: WakeRule) -> Self {
        Self {
            rule,
            queued: AtomicUsize::new(CAPACITY),
            items: AtomicUsize::new(CAPACITY),
            space: Notify::new(),
            arrived: Notify::new(),
        }
    }

    fn send(&self) {
        let mut waited = false;
        loop {
            if self.queued.fetch_add(1, Ordering::AcqRel) < CAPACITY {
                self.items.fetch_add(1, Ordering::AcqRel);
                self.arrived.notify();
                let room = self.queued.load(Ordering::Acquire) < CAPACITY;
                if self.rule == WakeRule::PassToNextSender && waited && room {
                    self.space.notify();
                }
                return;
            }
            self.queued.fetch_sub(1, Ordering::AcqRel);
            if self.queued.load(Ordering::Acquire) >= CAPACITY {
                self.space.wait();
                waited = true;
            }
        }
    }

    fn take_all(&self) -> usize {
        let mut taken = 0;
        while self.items.load(Ordering::Acquire) > 0 {
            self.items.fetch_sub(1, Ordering::AcqRel);
            if self.queued.fetch_sub(1, Ordering::AcqRel) >= CAPACITY {
                self.space.notify();
            }
            taken += 1;
        }
        taken
    }
}

fn drain_with_waiting_senders(rule: WakeRule) {
    loom::model(move || {
        let queue = Arc::new(Queue::new(rule));
        let senders: Vec<_> = (0..WAITING_SENDERS)
            .map(|_| {
                let queue = Arc::clone(&queue);
                thread::spawn(move || queue.send())
            })
            .collect();
        let mut taken = 0;
        while taken < CAPACITY + WAITING_SENDERS {
            let batch = queue.take_all();
            taken += batch;
            if batch == 0 {
                queue.arrived.wait();
            }
        }
        for sender in senders {
            sender.join().unwrap();
        }
    });
}

#[test]
fn every_waiting_sender_gets_room_after_a_batch_drain() {
    drain_with_waiting_senders(WakeRule::PassToNextSender);
}

#[test]
#[ignore = "aborts the process on the deadlock it finds"]
fn the_earlier_wake_rule_loses_a_sender() {
    drain_with_waiting_senders(WakeRule::OnlyAtCapacity);
}
