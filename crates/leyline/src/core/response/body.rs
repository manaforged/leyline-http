use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use tokio::io::{AsyncWrite, AsyncWriteExt};

use super::prefix::stop_reason;
use super::{Response, ResponseBody, StopReason};
use crate::core::body_stream::BodyStream;
use crate::core::deadline::{Deadline, within};
use crate::core::error::{Cause, Error, Result};
use crate::core::session::decompress::{
    BodyLimit, Decoder, body_read_error, buffer_decoded, content_codings,
};

impl Response {
    /// Reads the body as text in the charset from `content-type`, defaulting to UTF-8.
    #[cfg(feature = "charset")]
    pub async fn text(self) -> crate::core::Result<String> {
        self.text_with_charset("utf-8").await
    }

    /// Reads the body as UTF-8 text and replaces invalid bytes.
    #[cfg(not(feature = "charset"))]
    pub async fn text(self) -> crate::core::Result<String> {
        Ok(String::from_utf8_lossy(&self.bytes().await?).into_owned())
    }

    /// Reads the body as text in the charset from `content-type`, defaulting to `default_encoding`.
    #[cfg(feature = "charset")]
    pub async fn text_with_charset(self, default_encoding: &str) -> crate::core::Result<String> {
        let label = self.charset_label().map(str::to_owned);
        let bytes = self.bytes().await?;
        let encoding = encoding_rs::Encoding::for_label(
            label.as_deref().unwrap_or(default_encoding).as_bytes(),
        )
        .unwrap_or(encoding_rs::UTF_8);
        Ok(encoding.decode(&bytes).0.into_owned())
    }

    #[cfg(any(feature = "charset", feature = "html"))]
    fn content_type(&self) -> Option<(&str, &str)> {
        let ct = self.header("content-type")?;
        let (media, params) = ct.split_once(';').unwrap_or((ct, ""));
        Some((media.trim(), params))
    }

    #[cfg(feature = "html")]
    pub(crate) fn media_type(&self) -> Option<&str> {
        self.content_type().map(|(media, _)| media)
    }

    #[cfg(feature = "charset")]
    fn charset_label(&self) -> Option<&str> {
        let (_, params) = self.content_type()?;
        params.split(';').find_map(|param| {
            let (k, v) = param.split_once('=')?;
            k.trim()
                .eq_ignore_ascii_case("charset")
                .then(|| v.trim().trim_matches('"'))
        })
    }

    /// Reads the whole decoded body and fails when it exceeds the size limit.
    pub async fn bytes(self) -> crate::core::Result<Bytes> {
        let encoding = self.content_coding();
        let stream = match self.parts.body {
            ResponseBody::Buffered(b) => return Ok(Bytes::from(b)),
            ResponseBody::Taken => {
                return Err(Error::new(Cause::Body).with_message(
                    "response body stream was taken by `bytes_stream`; read the bytes from that stream",
                ));
            }
            ResponseBody::Streaming(s) => s,
        };
        let (buf, _) = buffer_decoded(stream, encoding.as_deref(), &self.parts.compression).await?;
        Ok(Bytes::from(buf))
    }

    /// Reads the body and deserializes it from JSON.
    pub async fn json<T: serde::de::DeserializeOwned>(self) -> crate::core::Result<T> {
        serde_json::from_slice(&self.bytes().await?).map_err(Error::from_json)
    }

    fn take_stream(mut self) -> Result<BodyStream> {
        match std::mem::replace(&mut self.parts.body, ResponseBody::Taken) {
            ResponseBody::Streaming(s) => Ok(s),
            ResponseBody::Buffered(b) => Ok(BodyStream::from_bytes(bytes::Bytes::from(b))),
            ResponseBody::Taken => Err(Error::new(Cause::Body)
                .with_message("response body has already been taken as a stream")),
        }
    }

    pub(super) fn content_coding(&self) -> Option<String> {
        content_codings(self.parts.headers.get_all(http::header::CONTENT_ENCODING))
    }

    /// Returns the decoded body as a stream and fails when the declared length exceeds the size limit.
    pub fn bytes_stream(
        self,
    ) -> Result<impl Stream<Item = Result<Bytes>> + Send + Unpin + 'static> {
        Ok(self
            .body_stream()?
            .map(|chunk| chunk.map_err(body_read_error)))
    }

    pub(crate) fn body_stream(self) -> Result<BodyStream> {
        let encoding = self.content_coding();
        if encoding.is_none() || !self.parts.compression.decode {
            self.refuse_declared_overflow()?;
        }
        let decoder = Decoder::capped(encoding.as_deref(), &self.parts.compression)?;
        Ok(BodyStream::decoded(self.take_stream()?, decoder))
    }

    fn refuse_declared_overflow(&self) -> Result<()> {
        let cap = BodyLimit::session(self.parts.compression.max_body_size);
        match self.content_length() {
            Some(declared) if declared > u64::try_from(cap.bytes).unwrap_or(u64::MAX) => {
                Err(cap.error())
            }
            _ => Ok(()),
        }
    }

    /// Writes the decoded body to `writer` and returns the number of bytes written.
    pub async fn write_to<W>(self, writer: &mut W) -> Result<u64>
    where
        W: AsyncWrite + Unpin,
    {
        write_stream(self.body_stream()?, writer, body_read_error).await
    }

    pub(crate) async fn error_for_status_with_body(self, deadline: &Deadline) -> Result<Self> {
        let Some(err) = self.status_error() else {
            return Ok(self);
        };
        let limit = self.parts.compression.max_error_body;
        let declared = self.content_length();
        let mut body = Vec::new();
        let mut decodes = false;
        let read = within(
            Some(deadline.error_body_wait()),
            self.read_prefix_into(limit, |_, _| false, &mut body, &mut decodes),
        )
        .await;
        if !matches!(read, Ok(Ok(_))) && body.is_empty() {
            return Err(err);
        }
        let complete = declared == Some(body.len() as u64);
        let err = err.with_body(body);
        Err(match (decodes, complete) {
            (true, _) => err.without_content_coding(),
            (false, false) => err.without_content_length(),
            (false, true) => err,
        })
    }

    /// Reads the body until `done` returns `true` or `limit` bytes arrive, and returns the bytes with `true` when `done` matched.
    pub async fn read_until<F>(self, limit: usize, done: F) -> Result<(Vec<u8>, bool)>
    where
        F: FnMut(&[u8], usize) -> bool,
    {
        let mut bytes = Vec::new();
        let stopped_by = self
            .read_prefix_into(limit, done, &mut bytes, &mut false)
            .await?;
        Ok((bytes, stopped_by == StopReason::PredicateMatched))
    }

    async fn read_prefix_into<F>(
        self,
        limit: usize,
        mut done: F,
        out: &mut Vec<u8>,
        decodes: &mut bool,
    ) -> Result<StopReason>
    where
        F: FnMut(&[u8], usize) -> bool,
    {
        let limit = limit.min(self.parts.compression.max_body_size);
        if limit == 0 {
            return Ok(StopReason::LimitReached);
        }
        let encoding = self.content_coding();
        let mut decoder = Decoder::truncated(encoding.as_deref(), &self.parts.compression, limit)?;
        *decodes = decoder.is_some();
        let mut stream = self.take_stream()?;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(Error::from)?;
            let from = out.len();
            match decoder.as_mut() {
                Some(decoder) => decoder.feed(&chunk, out)?,
                None => out.extend_from_slice(&chunk),
            }
            out.truncate(limit);
            if let Some(reason) = stop_reason(out, from, limit, &mut done) {
                return Ok(reason);
            }
        }
        let from = out.len();
        if let Some(decoder) = decoder.as_mut() {
            decoder.finish(out)?;
            out.truncate(limit);
        }
        Ok(stop_reason(out, from, limit, &mut done).unwrap_or(StopReason::EndOfBody))
    }
}

async fn write_stream<S, W>(
    mut stream: S,
    writer: &mut W,
    map: fn(std::io::Error) -> Error,
) -> Result<u64>
where
    S: Stream<Item = std::io::Result<Bytes>> + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut total: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(map)?;
        writer.write_all(&chunk).await.map_err(Error::from)?;
        total += chunk.len() as u64;
    }
    writer.flush().await.map_err(Error::from)?;
    Ok(total)
}
