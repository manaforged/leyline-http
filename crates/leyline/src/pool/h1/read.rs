use http::{HeaderMap, HeaderName, HeaderValue};

use super::*;

pub(super) const READ_CHUNK: usize = 8192;

pub(super) const MAX_H1_CHUNK_LINE_BYTES: usize = 16 * 1024;

pub(super) enum Step {
    Size(u64),
    Data(usize),
    Done,
}

#[derive(Clone, Copy)]
enum State {
    Size,
    Data(u64),
    Crlf,
    Trailers,
    Done,
}

pub(super) struct Chunked {
    state: State,
    trailer_bytes: usize,
    trailers: HeaderMap,
}

impl Chunked {
    pub(super) fn new() -> Self {
        Self {
            state: State::Size,
            trailer_bytes: 0,
            trailers: HeaderMap::new(),
        }
    }

    pub(super) async fn next<S>(
        &mut self,
        stream: &mut S,
        buf: &mut Vec<u8>,
    ) -> Result<Step, H1Error>
    where
        S: AsyncRead + Unpin + ?Sized,
    {
        loop {
            if let Some(step) = self.step(buf)? {
                return Ok(step);
            }
            read_more(stream, buf).await?;
        }
    }

    pub(super) fn into_trailers(self) -> HeaderMap {
        self.trailers
    }

    fn step(&mut self, buf: &mut Vec<u8>) -> Result<Option<Step>, Error> {
        loop {
            match self.state {
                State::Size => {
                    let Some(end) = line_end(buf)? else {
                        return Ok(None);
                    };
                    let size = chunk_size(&buf[..end]).map_err(malformed)?;
                    buf.drain(..end + 2);
                    if size == 0 {
                        self.state = State::Trailers;
                        continue;
                    }
                    self.state = State::Data(size);
                    return Ok(Some(Step::Size(size)));
                }
                State::Data(remaining) => {
                    if buf.is_empty() {
                        return Ok(None);
                    }
                    let take =
                        usize::try_from(remaining).map_or(buf.len(), |left| left.min(buf.len()));
                    let left = remaining - take as u64;
                    self.state = if left == 0 {
                        State::Crlf
                    } else {
                        State::Data(left)
                    };
                    return Ok(Some(Step::Data(take)));
                }
                State::Crlf => {
                    if buf.len() < 2 {
                        return Ok(None);
                    }
                    if &buf[..2] != b"\r\n" {
                        return Err(malformed("chunk missing CRLF terminator"));
                    }
                    buf.drain(..2);
                    self.state = State::Size;
                }
                State::Trailers => {
                    if !self.trailer_line(buf)? {
                        return Ok(None);
                    }
                }
                State::Done => return Ok(Some(Step::Done)),
            }
        }
    }

    fn trailer_line(&mut self, buf: &mut Vec<u8>) -> Result<bool, Error> {
        let Some(end) = line_end(buf)? else {
            return Ok(false);
        };
        self.trailer_bytes += end + 2;
        if self.trailer_bytes > MAX_H1_HEADER_BYTES {
            return Err(malformed(format!(
                "HTTP/1.1 trailers exceed {MAX_H1_HEADER_BYTES} bytes"
            )));
        }
        if end == 0 {
            self.state = State::Done;
        } else {
            self.trailer(&buf[..end]);
        }
        buf.drain(..end + 2);
        Ok(true)
    }

    fn trailer(&mut self, line: &[u8]) {
        let Some(colon) = memchr::memchr(b':', line) else {
            return;
        };
        let value = trim_end(trim_start(&line[colon + 1..]));
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(&line[..colon]),
            HeaderValue::from_bytes(value),
        ) {
            self.trailers.append(name, value);
        }
    }
}

fn line_end(buf: &[u8]) -> Result<Option<usize>, Error> {
    if let Some(pos) = memchr::memmem::find(buf, b"\r\n") {
        return Ok(Some(pos));
    }
    if buf.len() > MAX_H1_CHUNK_LINE_BYTES {
        return Err(malformed(format!(
            "chunked size/trailer line exceeds {MAX_H1_CHUNK_LINE_BYTES} bytes"
        )));
    }
    Ok(None)
}

pub(super) fn cut_short() -> H1Error {
    closed("before HTTP/1.1 body completed")
}

pub(super) async fn read_more<S>(stream: &mut S, buf: &mut Vec<u8>) -> Result<(), H1Error>
where
    S: AsyncRead + Unpin + ?Sized,
{
    if read_into(stream, buf, READ_CHUNK).await? == 0 {
        return Err(closed("during chunked body"));
    }
    Ok(())
}

pub(super) async fn read_into<S>(stream: &mut S, buf: &mut Vec<u8>, max: usize) -> io::Result<usize>
where
    S: AsyncRead + Unpin + ?Sized,
{
    buf.reserve(max);
    stream
        .read_buf(&mut bytes::BufMut::limit(&mut *buf, max))
        .await
}
