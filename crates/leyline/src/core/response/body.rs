use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use tokio::io::{AsyncWrite, AsyncWriteExt};

use super::prefix::stop_reason;
use super::{PrefixRead, Response, ResponseBody, StopReason};
use crate::core::body_stream::BodyStream;
use crate::core::deadline::{Deadline, within};
use crate::core::error::{Error, Kind, Result};
use crate::core::session::decompress::{
    BodyLimit, Decoder, body_read_error, content_codings, decompress_body, drain_stream_into_vec,
};

impl Response {
    pub async fn text(self) -> crate::core::Result<String> {
        self.text_with_charset("utf-8").await
    }

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

    #[cfg(not(feature = "charset"))]
    pub async fn text_with_charset(self, _default_encoding: &str) -> crate::core::Result<String> {
        Ok(String::from_utf8_lossy(&self.bytes().await?).into_owned())
    }

    #[cfg(feature = "charset")]
    fn charset_label(&self) -> Option<&str> {
        let ct = self.header("content-type")?;
        ct.split(';').skip(1).find_map(|param| {
            let (k, v) = param.split_once('=')?;
            k.trim()
                .eq_ignore_ascii_case("charset")
                .then(|| v.trim().trim_matches('"'))
        })
    }

    pub async fn bytes(self) -> crate::core::Result<Bytes> {
        let stream = match self.body {
            ResponseBody::Buffered(b) => return Ok(Bytes::from(b)),
            ResponseBody::Taken => {
                return Err(Error::new(Kind::Body).with_message(
                    "response body stream was taken by `into_stream`; read the bytes from that stream",
                ));
            }
            ResponseBody::Streaming(s) => s,
        };
        let buf = drain_stream_into_vec(stream, self.compression.max_body_size).await?;
        let encoding = content_codings(self.headers.get_all(http::header::CONTENT_ENCODING));
        let (buf, _) = decompress_body(buf, encoding.as_deref(), &self.compression)?;
        Ok(Bytes::from(buf))
    }

    pub async fn json<T: serde::de::DeserializeOwned>(self) -> crate::core::Result<T> {
        serde_json::from_slice(&self.bytes().await?).map_err(Error::from_json)
    }

    pub fn into_stream(mut self) -> Result<BodyStream> {
        match std::mem::replace(&mut self.body, ResponseBody::Taken) {
            ResponseBody::Streaming(s) => Ok(s),
            ResponseBody::Buffered(b) => Ok(BodyStream::from_bytes(bytes::Bytes::from(b))),
            ResponseBody::Taken => Err(Error::new(Kind::Body)
                .with_message("response body has already been taken as a stream")),
        }
    }

    pub async fn copy_to<W>(self, writer: &mut W) -> Result<u64>
    where
        W: tokio::io::AsyncWrite + Unpin,
    {
        write_stream(self.into_stream()?, writer, Error::from).await
    }

    pub fn into_decoded_stream(self, limit: Option<u64>) -> Result<BodyStream> {
        let encoding = content_codings(self.headers.get_all(http::header::CONTENT_ENCODING));
        if encoding.is_none() {
            self.refuse_declared_overflow(limit)?;
        }
        let decoder = Decoder::capped(encoding.as_deref(), &self.compression, limit)?;
        Ok(BodyStream::decoded(self.into_stream()?, decoder))
    }

    fn refuse_declared_overflow(&self, limit: Option<u64>) -> Result<()> {
        let cap = BodyLimit::session(self.compression.max_body_size).tighter(limit);
        match self.content_length() {
            Some(declared) if declared > u64::try_from(cap.bytes).unwrap_or(u64::MAX) => {
                Err(cap.error())
            }
            _ => Ok(()),
        }
    }

    pub async fn copy_decoded_to<W>(self, writer: &mut W, limit: Option<u64>) -> Result<u64>
    where
        W: AsyncWrite + Unpin,
    {
        write_stream(self.into_decoded_stream(limit)?, writer, body_read_error).await
    }

    pub(crate) async fn error_for_status_with_body(self, deadline: &Deadline) -> Result<Self> {
        let Some(err) = self.status_error() else {
            return Ok(self);
        };
        let limit = self.compression.max_error_body;
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

    pub async fn read_until<F>(self, limit: usize, done: F) -> Result<Vec<u8>>
    where
        F: FnMut(&[u8], usize) -> bool,
    {
        Ok(self.read_prefix(limit, done).await?.bytes)
    }

    pub(crate) async fn read_prefix<F>(self, limit: usize, done: F) -> Result<PrefixRead>
    where
        F: FnMut(&[u8], usize) -> bool,
    {
        let mut bytes = Vec::new();
        let stopped_by = self
            .read_prefix_into(limit, done, &mut bytes, &mut false)
            .await?;
        Ok(PrefixRead { bytes, stopped_by })
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
        let limit = limit.min(self.compression.max_body_size);
        if limit == 0 {
            return Ok(StopReason::LimitReached);
        }
        let encoding = content_codings(self.headers.get_all(http::header::CONTENT_ENCODING));
        let mut decoder = Decoder::truncated(encoding.as_deref(), &self.compression, limit)?;
        *decodes = decoder.is_some();
        let mut stream = self.into_stream()?;
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
