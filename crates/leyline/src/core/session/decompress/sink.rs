use std::io::Write;

#[derive(Debug)]
pub(super) struct Oversize(pub(super) usize);

impl std::fmt::Display for Oversize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "decompressed size exceeds {} bytes", self.0)
    }
}

impl std::error::Error for Oversize {}

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
            return Err(std::io::Error::other(Oversize(self.limit)));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
