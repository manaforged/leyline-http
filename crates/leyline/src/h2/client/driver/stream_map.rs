use std::collections::{HashMap, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::sync::mpsc::error::TrySendError;

use super::{PendingSend, ResponseSink, StreamActor};
use crate::h2::client::handle::Streams;

pub(super) struct StreamMap {
    inner: HashMap<u32, StreamActor, BuildHasherDefault<StreamIdHasher>>,
    open: Arc<Streams>,
    stalled: usize,
    parked: VecDeque<u32>,
}

#[derive(Default)]
struct StreamIdHasher(u64);

impl Hasher for StreamIdHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u32(b as u32);
        }
    }

    fn write_u32(&mut self, n: u32) {
        self.0 = (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }

    fn write_usize(&mut self, n: usize) {
        self.0 = (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }

    fn finish(&self) -> u64 {
        self.0 >> 33
    }
}

impl StreamMap {
    pub(super) fn new(open: Arc<Streams>) -> Self {
        Self {
            inner: HashMap::default(),
            open,
            stalled: 0,
            parked: VecDeque::new(),
        }
    }

    fn publish(&self) {
        self.open.open.store(self.inner.len(), Ordering::Relaxed);
    }

    pub(super) fn set_capacity(&self, capacity: usize) {
        self.open.capacity.store(capacity, Ordering::Relaxed);
    }

    pub(super) fn has_stalled(&self) -> bool {
        self.stalled > 0
    }

    pub(super) fn add_stalled(&mut self, chunks: usize) {
        self.stalled += chunks;
    }

    pub(super) fn park(&mut self, stream_id: u32, pending: PendingSend) {
        if let Some(actor) = self.inner.get_mut(&stream_id) {
            actor.pending_send = Some(pending);
            if !self.parked.contains(&stream_id) {
                self.parked.push_back(stream_id);
            }
        }
    }

    pub(super) fn unpark(&mut self, stream_id: u32) {
        self.parked.retain(|&id| id != stream_id);
    }

    pub(super) fn parked(&self) -> usize {
        self.parked.len()
    }

    pub(super) fn next_parked(&mut self) -> Option<u32> {
        self.parked.pop_front()
    }

    pub(super) fn flush_stalled(&mut self) -> Vec<(u32, bool)> {
        let mut drained = Vec::new();
        for (&sid, actor) in self.inner.iter_mut() {
            let Some(ResponseSink::StreamingEx { body_tx, .. }) = actor.response_tx.as_ref() else {
                continue;
            };
            while let Some(chunk) = actor.stalled.pop_front() {
                self.stalled -= 1;
                match body_tx.try_send(Ok(chunk)) {
                    Ok(()) => {}
                    Err(TrySendError::Full(Ok(chunk))) => {
                        actor.stalled.push_front(chunk);
                        self.stalled += 1;
                        break;
                    }
                    Err(_) => break,
                }
            }
            if actor.stalled.is_empty() {
                drained.push((sid, actor.remote_done));
            }
        }
        drained
    }

    pub(super) fn get(&self, stream_id: u32) -> Option<&StreamActor> {
        self.inner.get(&stream_id)
    }

    pub(super) fn get_mut(&mut self, stream_id: u32) -> Option<&mut StreamActor> {
        self.inner.get_mut(&stream_id)
    }

    pub(super) fn insert(&mut self, stream_id: u32, actor: StreamActor) {
        self.inner.insert(stream_id, actor);
        self.publish();
    }

    pub(super) fn remove(&mut self, stream_id: u32) -> Option<StreamActor> {
        let actor = self.inner.remove(&stream_id);
        if let Some(actor) = &actor {
            self.stalled = self.stalled.saturating_sub(actor.stalled.len());
        }
        self.unpark(stream_id);
        self.publish();
        actor
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (u32, &StreamActor)> {
        self.inner.iter().map(|(&id, actor)| (id, actor))
    }

    pub(super) fn keys(&self) -> impl Iterator<Item = &u32> {
        self.inner.keys()
    }

    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut StreamActor)> {
        self.inner.iter_mut().map(|(&id, actor)| (id, actor))
    }

    pub(super) fn drain(&mut self) -> impl Iterator<Item = (u32, StreamActor)> {
        self.open.open.store(0, Ordering::Relaxed);
        self.stalled = 0;
        self.parked.clear();
        self.inner.drain()
    }

    pub(super) fn values(&self) -> impl Iterator<Item = &StreamActor> {
        self.inner.values()
    }

    pub(super) fn len(&self) -> usize {
        self.inner.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}
