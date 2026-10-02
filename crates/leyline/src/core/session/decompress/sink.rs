use std::io::Write;

use super::limit::{Cap, Overflow};

pub(super) struct Sink {
    pub(super) buf: Vec<u8>,
    written: usize,
    cap: Cap,
    truncated: bool,
}

impl Sink {
    pub(super) fn new(cap: Cap) -> Self {
        Self {
            buf: Vec::new(),
            written: 0,
            cap,
            truncated: false,
        }
    }

    pub(super) fn truncated(&self) -> bool {
        self.truncated
    }
}

impl Write for Sink {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let room = self.cap.room(self.written);
        self.written = self.written.saturating_add(data.len());
        if data.len() > room {
            if self.cap.overflow == Overflow::Truncate {
                self.buf.extend_from_slice(&data[..room]);
                self.truncated = true;
            }
            return Err(self.cap.limit.into_io());
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
