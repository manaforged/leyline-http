use std::io;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::{Stream, TryFutureExt};

use crate::core::body::{Body, BodyKind};

mod debug;

pub struct Part {
    pub(crate) name: String,
    pub(crate) body: Body,
    pub(crate) filename: Option<String>,
    pub(crate) mime: Option<String>,
    pub(crate) extra_headers: Vec<(String, String)>,
}

impl Part {
    pub fn text(value: impl Into<String>) -> Self {
        let s = value.into();
        Self {
            name: String::new(),
            body: Body::bytes(Bytes::from(s.into_bytes())),
            filename: None,
            mime: Some("text/plain; charset=utf-8".into()),
            extra_headers: Vec::new(),
        }
    }

    pub fn bytes(bytes: impl Into<Bytes>) -> Self {
        Self {
            name: String::new(),
            body: Body::bytes(bytes.into()),
            filename: None,
            mime: None,
            extra_headers: Vec::new(),
        }
    }

    pub fn stream<S>(stream: S) -> Self
    where
        S: Stream<Item = io::Result<Bytes>> + Send + 'static,
    {
        Self {
            name: String::new(),
            body: Body::stream(stream, None),
            filename: None,
            mime: None,
            extra_headers: Vec::new(),
        }
    }

    pub fn file(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let metadata = std::fs::metadata(&path)?;
        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file")
            .to_string();
        let stream = tokio::fs::File::open(path)
            .map_ok(tokio_util::io::ReaderStream::new)
            .try_flatten_stream();
        Ok(Self {
            name: String::new(),
            body: Body::stream(stream, Some(metadata.len())),
            filename: Some(filename),
            mime: None,
            extra_headers: Vec::new(),
        })
    }

    pub fn filename(mut self, name: impl Into<String>) -> Self {
        self.filename = Some(name.into());
        self
    }

    pub fn mime(mut self, mime: impl Into<String>) -> Self {
        self.mime = Some(mime.into());
        self
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((name.into(), value.into()));
        self
    }
}

pub struct Form {
    pub(crate) parts: Vec<Part>,
    pub(crate) boundary: String,
}

impl Default for Form {
    fn default() -> Self {
        Self::new()
    }
}

impl Form {
    pub fn new() -> Self {
        Self {
            parts: Vec::new(),
            boundary: random_boundary(),
        }
    }

    pub fn boundary(&self) -> &str {
        &self.boundary
    }

    pub fn text(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let mut part = Part::text(value);
        part.name = name.into();
        self.parts.push(part);
        self
    }

    pub fn part(mut self, name: impl Into<String>, mut part: Part) -> Self {
        part.name = name.into();
        self.parts.push(part);
        self
    }

    pub fn file(mut self, name: impl Into<String>, path: impl AsRef<Path>) -> io::Result<Self> {
        let mut part = Part::file(path)?;
        part.name = name.into();
        self.parts.push(part);
        Ok(self)
    }

    pub(crate) fn len_hint(&self) -> Option<u64> {
        let mut total: u64 = 0;
        for part in &self.parts {
            total += 2 + self.boundary.len() as u64 + 2;
            total += part_header_len(part) as u64;
            let body_len = part.body.len_hint()?;
            total += body_len;
            total += 2;
        }
        total += 2 + self.boundary.len() as u64 + 4;
        Some(total)
    }

    pub(crate) fn into_stream_body(self) -> Body {
        if let Some(bytes) = self.buffered() {
            return Body::bytes(bytes);
        }
        let length_hint = self.len_hint();
        Body::stream(FormStream::new(self), length_hint)
    }

    fn buffered(&self) -> Option<Bytes> {
        let mut out = Vec::with_capacity(usize::try_from(self.len_hint()?).ok()?);
        for part in &self.parts {
            let body: &[u8] = match &part.body.0 {
                BodyKind::Empty => &[],
                BodyKind::Bytes(b) => b,
                BodyKind::Stream { .. } => return None,
            };
            out.extend_from_slice(&render_part_headers(&self.boundary, part).ok()?);
            out.extend_from_slice(body);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"--");
        out.extend_from_slice(self.boundary.as_bytes());
        out.extend_from_slice(b"--\r\n");
        Some(Bytes::from(out))
    }

    pub(crate) fn content_type(&self) -> String {
        format!("multipart/form-data; boundary={}", self.boundary)
    }
}

impl From<Form> for Body {
    fn from(form: Form) -> Body {
        form.into_stream_body()
    }
}

fn part_header_len(part: &Part) -> usize {
    let mut n = "Content-Disposition: form-data; name=\"\"\r\n".len() + quoted_len(&part.name);
    if let Some(fname) = &part.filename {
        n += "; filename=\"\"".len() + quoted_len(fname);
    }
    if let Some(mime) = &part.mime {
        n += "Content-Type: \r\n".len() + mime.len();
    }
    for (k, v) in &part.extra_headers {
        n += k.len() + 2 + v.len() + 2;
    }
    n += 2;
    n
}

struct FormStream {
    boundary: String,
    parts: std::collections::VecDeque<Part>,
    state: FormState,
}

enum FormState {
    NextPart,
    InBody(Pin<Box<dyn Stream<Item = io::Result<Bytes>> + Send + 'static>>),
    PartEnd,
    Done,
}

impl FormStream {
    fn new(form: Form) -> Self {
        Self {
            boundary: form.boundary,
            parts: form.parts.into(),
            state: FormState::NextPart,
        }
    }
}

fn quoted_len(input: &str) -> usize {
    input.len() + input.bytes().filter(|b| matches!(b, b'"' | b'\\')).count()
}

fn escape_quoted(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() + 2);
    for b in input.bytes() {
        match b {
            0x00..=0x1F | 0x7F => return None,
            b'"' | b'\\' => {
                out.push(b'\\');
                out.push(b);
            }
            _ => out.push(b),
        }
    }
    Some(out)
}

fn render_part_headers(boundary: &str, part: &Part) -> io::Result<Bytes> {
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(b"--");
    out.extend_from_slice(boundary.as_bytes());
    out.extend_from_slice(b"\r\n");

    out.extend_from_slice(b"Content-Disposition: form-data; name=\"");
    let escaped_name = escape_quoted(&part.name).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "multipart part `name` contains control characters (CRLF/NUL/etc.)",
        )
    })?;
    out.extend_from_slice(&escaped_name);
    out.push(b'"');
    if let Some(fname) = &part.filename {
        let escaped = escape_quoted(fname).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "multipart part `filename` contains control characters (CRLF/NUL/etc.)",
            )
        })?;
        out.extend_from_slice(b"; filename=\"");
        out.extend_from_slice(&escaped);
        out.push(b'"');
    }
    out.extend_from_slice(b"\r\n");

    if let Some(mime) = &part.mime {
        if mime.bytes().any(|b| matches!(b, 0x00..=0x1F | 0x7F)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "multipart part `mime` contains control characters",
            ));
        }
        out.extend_from_slice(b"Content-Type: ");
        out.extend_from_slice(mime.as_bytes());
        out.extend_from_slice(b"\r\n");
    }

    for (k, v) in &part.extra_headers {
        let bad = k.bytes().any(|b| matches!(b, 0x00..=0x1F | 0x7F))
            || v.bytes().any(|b| matches!(b, 0x00..=0x0A | 0x0D | 0x7F));
        if bad {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "multipart part extra header contains control characters",
            ));
        }
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(v.as_bytes());
        out.extend_from_slice(b"\r\n");
    }

    out.extend_from_slice(b"\r\n");
    Ok(Bytes::from(out))
}

impl Stream for FormStream {
    type Item = io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // SAFETY: we never move `self.state` variants that hold a `Pin<Box<...>>` out of the structure — we only poll them in place via `.as_mut()`.
            let this = &mut *self;
            match &mut this.state {
                FormState::NextPart => {
                    let Some(part) = this.parts.pop_front() else {
                        let mut out = Vec::with_capacity(this.boundary.len() + 6);
                        out.extend_from_slice(b"--");
                        out.extend_from_slice(this.boundary.as_bytes());
                        out.extend_from_slice(b"--\r\n");
                        this.state = FormState::Done;
                        return Poll::Ready(Some(Ok(Bytes::from(out))));
                    };
                    let headers = match render_part_headers(&this.boundary, &part) {
                        Ok(h) => h,
                        Err(e) => {
                            this.state = FormState::Done;
                            return Poll::Ready(Some(Err(e)));
                        }
                    };
                    let body = part.body;
                    let body_stream: Pin<
                        Box<dyn Stream<Item = io::Result<Bytes>> + Send + 'static>,
                    > = match body.0 {
                        BodyKind::Empty => Box::pin(futures_util::stream::empty()),
                        BodyKind::Bytes(b) => {
                            Box::pin(futures_util::stream::once(async move { Ok(b) }))
                        }
                        BodyKind::Stream { stream, .. } => stream,
                    };
                    this.state = FormState::InBody(body_stream);
                    return Poll::Ready(Some(Ok(headers)));
                }
                FormState::InBody(stream) => match stream.as_mut().poll_next(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Some(Ok(b))) => return Poll::Ready(Some(Ok(b))),
                    Poll::Ready(Some(Err(e))) => {
                        this.state = FormState::Done;
                        return Poll::Ready(Some(Err(e)));
                    }
                    Poll::Ready(None) => {
                        this.state = FormState::PartEnd;
                        continue;
                    }
                },
                FormState::PartEnd => {
                    this.state = FormState::NextPart;
                    return Poll::Ready(Some(Ok(Bytes::from_static(b"\r\n"))));
                }
                FormState::Done => return Poll::Ready(None),
            }
        }
    }
}

fn random_boundary() -> String {
    format!(
        "----LeylineFormBoundary{}",
        crate::util::random_hex_token(16)
    )
}

#[cfg(test)]
mod tests;
