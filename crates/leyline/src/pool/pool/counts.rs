use crate::pool::Pool;
use crate::pool::types::PooledConn;
use crate::util::lock;

#[derive(Default)]
pub(super) struct Counts {
    pub(super) busy: usize,
    pub(super) idle: usize,
}

impl Counts {
    fn add(&mut self, busy: bool) {
        if busy {
            self.busy += 1;
        } else {
            self.idle += 1;
        }
    }
}

impl Pool {
    pub(super) fn connection_counts(&self) -> Counts {
        let mut counts = Counts::default();
        for conn in lock(&self.inner).values() {
            match conn {
                PooledConn::H1 { idle, .. } => counts.idle += idle.len(),
                PooledConn::H2 { handle, .. } => counts.add(handle.open_streams() > 0),
                #[cfg(feature = "http3")]
                PooledConn::H3 { handle, .. } => counts.add(handle.open_streams() > 0),
            }
        }
        counts.busy += self.h1_checked_out();
        counts
    }

    fn h1_checked_out(&self) -> usize {
        lock(&self.h1_permits)
            .values()
            .map(|sem| {
                self.max_h1_conns_per_host
                    .saturating_sub(sem.available_permits())
            })
            .sum()
    }
}
