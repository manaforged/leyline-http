use super::*;

pub(super) enum Encoder {
    Fixed,
    Chunked,
}

impl Encoder {
    pub(super) fn encode(&mut self, chunk: Bytes, out: &mut VecDeque<Bytes>) {
        match self {
            Encoder::Fixed => {
                if !chunk.is_empty() {
                    out.push_back(chunk);
                }
            }
            Encoder::Chunked => {
                if !chunk.is_empty() {
                    out.push_back(Bytes::from(format!("{:X}\r\n", chunk.len())));
                    out.push_back(chunk);
                    out.push_back(Bytes::from_static(b"\r\n"));
                }
            }
        }
    }

    pub(super) fn finish(&self, out: &mut VecDeque<Bytes>) {
        match self {
            Encoder::Fixed => {}
            Encoder::Chunked => out.push_back(Bytes::from_static(b"0\r\n\r\n")),
        }
    }
}
