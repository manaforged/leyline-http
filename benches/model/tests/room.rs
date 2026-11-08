use loom::sync::atomic::{AtomicBool, AtomicUsize, Ordering, fence};
use loom::sync::{Arc, Mutex, Notify};
use loom::thread;

struct Room {
    watched: AtomicBool,
    wakers: Mutex<Vec<Arc<Notify>>>,
}

impl Room {
    fn freed(&self) {
        fence(Ordering::SeqCst);
        if !self.watched.load(Ordering::SeqCst) {
            return;
        }
        let wakers = {
            let mut wakers = self.wakers.lock().unwrap();
            self.watched.store(false, Ordering::SeqCst);
            std::mem::take(&mut *wakers)
        };
        for waker in wakers {
            waker.notify();
        }
    }

    fn watch(&self, waker: &Arc<Notify>) {
        let mut wakers = self.wakers.lock().unwrap();
        wakers.push(Arc::clone(waker));
        self.watched.store(true, Ordering::SeqCst);
        drop(wakers);
        fence(Ordering::SeqCst);
    }
}

#[test]
fn a_blocked_caller_sees_room_freed_while_it_registers() {
    loom::model(|| {
        let room = Arc::new(Room {
            watched: AtomicBool::new(false),
            wakers: Mutex::new(Vec::new()),
        });
        let free = Arc::new(AtomicUsize::new(0));
        let releasing = {
            let (room, free) = (Arc::clone(&room), Arc::clone(&free));
            thread::spawn(move || {
                free.store(1, Ordering::SeqCst);
                room.freed();
            })
        };
        let waker = Arc::new(Notify::new());
        loop {
            room.watch(&waker);
            if free.load(Ordering::SeqCst) == 1 {
                break;
            }
            waker.wait();
        }
        releasing.join().unwrap();
    });
}
