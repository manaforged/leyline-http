//! `multipart/form-data` bodies (RFC 7578).
//!
//! A `Form` carries an ordered list of `Part`s plus a random boundary
//! string. When the builder wires the form into a request, the body is
//! materialised as a streaming `Body::Stream` so very large file
//! uploads never land in memory all at once — each part's body is
//! pumped to the wire as the transport pulls it.
//!
//! Each part has a name, an optional filename and mime type, optional
//! extra headers, and a body. The body can be a text string, a raw
//! byte buffer, or another stream (for file-backed uploads).
//!
//! ```rust,ignore
//! use leyline::multipart::{Form, Part};
//!
//! let form = Form::new()
//!     .text("username", "alice")
//!     .part("avatar", Part::bytes(jpeg_bytes).filename("cat.jpg").mime("image/jpeg"));
//!
//! session.post(url).multipart(form).send().await?;
//! ```

use std::io;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::Stream;

use crate::body::Body;

/// A single part of a multipart form.
pub struct Part {
    pub(crate) name: String,
    pub(crate) body: Body,
    pub(crate) filename: Option<String>,
    pub(crate) mime: Option<String>,
    pub(crate) extra_headers: Vec<(String, String)>,
}

impl Part {
    /// Build a text part with `Content-Type: text/plain; charset=utf-8`.
    pub fn text(value: impl Into<String>) -> Self {
        let s = value.into();
        Self {
            name: String::new(),
            body: Body::Bytes(Bytes::from(s.into_bytes())),
            filename: None,
            mime: Some("text/plain; charset=utf-8".into()),
            extra_headers: Vec::new(),
        }
    }

    /// Build a part from a raw byte buffer. Defaults to no MIME type —
    /// set one with [`Part::mime`] to control how the server classifies
    /// the upload.
    pub fn bytes(bytes: impl Into<Bytes>) -> Self {
        Self {
            name: String::new(),
            body: Body::Bytes(bytes.into()),
            filename: None,
            mime: None,
            extra_headers: Vec::new(),
        }
    }

    /// Build a part from an arbitrary `Stream` yielding
    /// `io::Result<Bytes>`. The underlying stream is consumed once and
    /// is not replayable — streaming parts cannot participate in
    /// retries.
    pub fn stream<S>(stream: S) -> Self
    where
        S: Stream<Item = io::Result<Bytes>> + Send + 'static,
    {
        Self {
            name: String::new(),
            body: Body::stream(stream),
            filename: None,
            mime: None,
            extra_headers: Vec::new(),
        }
    }

    /// Set the `filename` attribute on this part's
    /// `Content-Disposition` header.
    pub fn filename(mut self, name: impl Into<String>) -> Self {
        self.filename = Some(name.into());
        self
    }

    /// Set the `Content-Type` header for this part.
    pub fn mime(mut self, mime: impl Into<String>) -> Self {
        self.mime = Some(mime.into());
        self
    }

    /// Append an extra header to this part (e.g. `Content-Transfer-Encoding`).
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((name.into(), value.into()));
        self
    }
}

/// A `multipart/form-data` form. Build with [`Form::new`], add parts
/// via [`Form::text`], [`Form::part`], or [`Form::file`], and wire it
/// into a request with
/// [`crate::RequestBuilder::multipart`].
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
    /// Create an empty form with a fresh random boundary.
    pub fn new() -> Self {
        Self {
            parts: Vec::new(),
            boundary: random_boundary(),
        }
    }

    /// The generated boundary string. Callers don't usually need this —
    /// the request builder wires the boundary into the `Content-Type`
    /// header for them.
    pub fn boundary(&self) -> &str {
        &self.boundary
    }

    /// Append a plain-text part with the given name and value.
    pub fn text(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let mut part = Part::text(value);
        part.name = name.into();
        self.parts.push(part);
        self
    }

    /// Append an arbitrary part with the given name.
    pub fn part(mut self, name: impl Into<String>, mut part: Part) -> Self {
        part.name = name.into();
        self.parts.push(part);
        self
    }

    /// Append a file part. Reads the file synchronously into memory —
    /// use [`Form::part`] with [`Part::stream`] for a streaming
    /// alternative on very large files.
    pub fn file(mut self, name: impl Into<String>, path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)?;
        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file")
            .to_string();
        let part = Part::bytes(bytes).filename(filename);
        self.parts.push(Part {
            name: name.into(),
            ..part
        });
        Ok(self)
    }

    /// Total length of the serialised body in bytes, when every part
    /// has a known length. Returns `None` if any part is a
    /// length-unknown stream.
    pub(crate) fn len_hint(&self) -> Option<u64> {
        let mut total: u64 = 0;
        for part in &self.parts {
            // Per-part boundary line: `--BOUNDARY\r\n`
            total += 2 + self.boundary.len() as u64 + 2;
            // Headers: Content-Disposition + optional mime + extras + blank line.
            total += part_header_len(part) as u64;
            let body_len = part.body.len_hint()?;
            total += body_len;
            total += 2; // trailing CRLF after part body
        }
        total += 2 + self.boundary.len() as u64 + 4; // final --BOUNDARY--\r\n
        Some(total)
    }

    /// Consume this form and produce a [`Body::Stream`] that yields the
    /// serialised multipart payload.
    pub(crate) fn into_stream_body(self) -> Body {
        let length_hint = self.len_hint();
        let stream = FormStream::new(self);
        if let Some(len) = length_hint {
            Body::stream_with_length(stream, len)
        } else {
            Body::stream(stream)
        }
    }

    /// The wire-format `Content-Type` header for this form, including
    /// the boundary parameter.
    pub fn content_type(&self) -> String {
        format!("multipart/form-data; boundary={}", self.boundary)
    }
}

fn part_header_len(part: &Part) -> usize {
    // `Content-Disposition: form-data; name="NAME"[; filename="FN"]\r\n`
    let mut n = "Content-Disposition: form-data; name=\"\"\r\n".len() + part.name.len();
    if let Some(fname) = &part.filename {
        n += "; filename=\"\"".len() + fname.len();
    }
    if let Some(mime) = &part.mime {
        n += "Content-Type: \r\n".len() + mime.len();
    }
    for (k, v) in &part.extra_headers {
        n += k.len() + 2 + v.len() + 2; // "k: v\r\n"
    }
    n += 2; // blank line between headers and body
    n
}

/// Stream adapter that walks through each part, emitting headers,
/// then pulling the part body, then the separator.
struct FormStream {
    boundary: String,
    parts: std::collections::VecDeque<Part>,
    state: FormState,
}

enum FormState {
    /// Emit the leading `--boundary\r\n` + part headers for the next part.
    NextPart,
    /// Drain the current part's body.
    InBody(Pin<Box<dyn Stream<Item = io::Result<Bytes>> + Send + 'static>>),
    /// Emit the trailing `\r\n` separator before the next part.
    PartEnd,
    /// Stream is complete.
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

fn render_part_headers(boundary: &str, part: &Part) -> Bytes {
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(b"--");
    out.extend_from_slice(boundary.as_bytes());
    out.extend_from_slice(b"\r\n");

    out.extend_from_slice(b"Content-Disposition: form-data; name=\"");
    out.extend_from_slice(part.name.as_bytes());
    out.push(b'"');
    if let Some(fname) = &part.filename {
        out.extend_from_slice(b"; filename=\"");
        out.extend_from_slice(fname.as_bytes());
        out.push(b'"');
    }
    out.extend_from_slice(b"\r\n");

    if let Some(mime) = &part.mime {
        out.extend_from_slice(b"Content-Type: ");
        out.extend_from_slice(mime.as_bytes());
        out.extend_from_slice(b"\r\n");
    }

    for (k, v) in &part.extra_headers {
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(v.as_bytes());
        out.extend_from_slice(b"\r\n");
    }

    out.extend_from_slice(b"\r\n");
    Bytes::from(out)
}

impl Stream for FormStream {
    type Item = io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // SAFETY: we never move `self.state` variants that hold a
            // `Pin<Box<...>>` out of the structure — we only poll them
            // in place via `.as_mut()`.
            let this = &mut *self;
            match &mut this.state {
                FormState::NextPart => {
                    let Some(part) = this.parts.pop_front() else {
                        // No more parts — emit closing boundary.
                        let mut out = Vec::with_capacity(this.boundary.len() + 6);
                        out.extend_from_slice(b"--");
                        out.extend_from_slice(this.boundary.as_bytes());
                        out.extend_from_slice(b"--\r\n");
                        this.state = FormState::Done;
                        return Poll::Ready(Some(Ok(Bytes::from(out))));
                    };
                    let headers = render_part_headers(&this.boundary, &part);
                    // Move the part body out so we can take ownership.
                    let body = part.body;
                    // Transition into InBody with a stream-view of the body.
                    let body_stream: Pin<
                        Box<dyn Stream<Item = io::Result<Bytes>> + Send + 'static>,
                    > = match body {
                        Body::Empty => Box::pin(futures_util::stream::empty()),
                        Body::Bytes(b) => {
                            Box::pin(futures_util::stream::once(async move { Ok(b) }))
                        }
                        Body::Stream { stream, .. } => stream,
                    };
                    this.state = FormState::InBody(body_stream);
                    return Poll::Ready(Some(Ok(headers)));
                }
                FormState::InBody(stream) => {
                    match stream.as_mut().poll_next(cx) {
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
                    }
                }
                FormState::PartEnd => {
                    this.state = FormState::NextPart;
                    return Poll::Ready(Some(Ok(Bytes::from_static(b"\r\n"))));
                }
                FormState::Done => return Poll::Ready(None),
            }
        }
    }
}

/// Produce a 48-hex-char random-looking boundary without pulling in
/// `rand`. Blends a per-call nanosecond timestamp, the process PID, a
/// monotonic counter, and the memory address of a fresh allocation.
fn random_boundary() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    // Use the address of a fresh local allocation as additional entropy.
    let marker: Box<u8> = Box::new(0);
    let addr = (&*marker) as *const u8 as u64;

    let a = now ^ addr.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let b = pid ^ counter.wrapping_mul(0xbf58_476d_1ce4_e5b9);

    format!("----LeylineFormBoundary{:016x}{:016x}", a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_are_distinct_between_forms() {
        let a = Form::new();
        let b = Form::new();
        assert_ne!(a.boundary(), b.boundary());
        assert!(a.boundary().starts_with("----LeylineFormBoundary"));
    }

    #[test]
    fn content_type_carries_boundary() {
        let f = Form::new();
        let ct = f.content_type();
        assert!(ct.starts_with("multipart/form-data; boundary=----LeylineFormBoundary"));
        assert!(ct.contains(f.boundary()));
    }
}
