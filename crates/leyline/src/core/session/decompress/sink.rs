use std::io::Write;

use super::BodyLimit;

pub(super) struct Sink {
    pub(super) buf: Vec<u8>,
    written: usize,
    limit: usize,
}

impl Sink {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            buf: Vec::new(),
            written: 0,
            limit,
        }
    }
}

impl Write for Sink {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.written = self.written.saturating_add(data.len());
        if self.written > self.limit {
            return Err(BodyLimit::session(self.limit).into_io());
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
