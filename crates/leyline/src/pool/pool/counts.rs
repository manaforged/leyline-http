use crate::pool::Pool;
use crate::util::lock;

#[derive(Default)]
pub(super) struct Counts {
    pub(super) busy: usize,
    pub(super) idle: usize,
}

impl Pool {
    pub(super) fn connection_counts(&self) -> Counts {
        let mut counts = Counts::default();
        for shard in self.conns.shards.iter() {
            let entries = crate::util::read(&shard.entries);
            counts.idle += entries
                .h1
                .values()
                .map(|entry| entry.idle.len())
                .sum::<usize>();
            for entry in entries.mux.values() {
                if entry.handle.open_streams() > 0 {
                    counts.busy += 1;
                } else {
                    counts.idle += 1;
                }
            }
        }
        counts.busy += self.h1_checked_out();
        counts
    }

    fn h1_checked_out(&self) -> usize {
        self.conns
            .shards
            .iter()
            .map(|shard| {
                lock(&shard.h1_permits)
                    .values()
                    .map(|slots| {
                        self.config
                            .max_h1_conns_per_host
                            .saturating_sub(slots.permits.available_permits())
                    })
                    .sum::<usize>()
            })
            .sum()
    }
}
