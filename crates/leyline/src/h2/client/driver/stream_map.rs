use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use super::StreamActor;

pub(super) struct StreamMap {
    inner: HashMap<u32, StreamActor, BuildHasherDefault<StreamIdHasher>>,
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
    pub(super) fn new() -> Self {
        Self {
            inner: HashMap::default(),
        }
    }

    pub(super) fn get(&self, stream_id: u32) -> Option<&StreamActor> {
        self.inner.get(&stream_id)
    }

    pub(super) fn get_mut(&mut self, stream_id: u32) -> Option<&mut StreamActor> {
        self.inner.get_mut(&stream_id)
    }

    pub(super) fn insert(&mut self, stream_id: u32, actor: StreamActor) {
        self.inner.insert(stream_id, actor);
    }

    pub(super) fn remove(&mut self, stream_id: u32) -> Option<StreamActor> {
        self.inner.remove(&stream_id)
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
