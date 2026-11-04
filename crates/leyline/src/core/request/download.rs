use std::path::Path;
use std::sync::Arc;

use futures_util::StreamExt;
use http::{Method, StatusCode, header};
use sha2::{Digest, Sha256};

use super::RequestBuilder;
use crate::core::saved::FileVersion;
use crate::core::{Error, Kind, Response, Result};
use crate::util::atomic::CallerWaits;

mod file;
use file::{Metadata, Partial};

type Progress = Arc<dyn Fn(u64, Option<u64>) + Send + Sync>;

pub(super) struct Download {
    checksum: Option<[u8; 32]>,
    progress: Option<Progress>,
    resume: bool,
}

impl Default for Download {
    fn default() -> Self {
        Self {
            checksum: None,
            progress: None,
            resume: true,
        }
    }
}

impl RequestBuilder {
    /// Verifies the SHA-256 of a download before replacing the destination file.
    pub fn sha256(mut self, checksum: [u8; 32]) -> Self {
        self.draft.download.get_or_insert_default().checksum = Some(checksum);
        self
    }
    /// Reports bytes written and the optional complete length during a download.
    pub fn progress(mut self, progress: impl Fn(u64, Option<u64>) + Send + Sync + 'static) -> Self {
        self.draft.download.get_or_insert_default().progress = Some(Arc::new(progress));
        self
    }
    /// Enables resuming a prior download, on by default for RequestBuilder::download_to.
    pub fn resume(mut self, resume: bool) -> Self {
        self.draft.download.get_or_insert_default().resume = resume;
        self
    }
    /// Downloads atomically and resumes a saved prefix only when a strong ETag still matches.
    pub async fn download_to(mut self, path: impl AsRef<Path>) -> Result<u64> {
        let key = self.prepare_download()?;
        let options = self.draft.download.take().unwrap_or_default();
        let target = path.as_ref().to_path_buf();
        let caller = CallerWaits::default();
        let resume = options.resume;
        let partial = Arc::new(blocking(move || Partial::open(target, key, resume)).await?);
        let old = partial.metadata();
        let offset = old.as_ref().map_or(0, |metadata| metadata.offset);
        if offset > self.draft.body_rules.max_body_size as u64 {
            return Err(limit());
        }
        self.resume_headers(old.as_ref())?;
        let response = self.send().await?;
        let (metadata, append) = metadata(&response, key, old.as_ref())?;
        if metadata
            .total
            .is_some_and(|total| total > response.parts.compression.max_body_size as u64)
        {
            return Err(limit());
        }
        let copy = metadata.clone();
        let part = Arc::clone(&partial);
        blocking(move || part.begin(copy, append)).await?;
        transfer(response, partial, metadata, options, caller).await
    }

    fn prepare_download(&mut self) -> Result<[u8; 32]> {
        if self.draft.method != Method::GET
            || self.draft.body.len_hint() != Some(0)
            || self.draft.headers.get(header::RANGE.as_str()).is_some()
            || self.draft.headers.get(header::IF_RANGE.as_str()).is_some()
        {
            return Err(Error::config(
                "download_to requires GET without a body or caller range headers",
            ));
        }
        self.draft
            .headers
            .set(header::ACCEPT_ENCODING, "identity")?;
        self.draft.body_rules.decode = false;
        self.draft.stream_response = true;
        let url = self.prepare()?;
        self.draft.url = Some(url);
        self.draft.query_params.clear();
        self.session.download_key(&self.draft)
    }

    fn resume_headers(&mut self, metadata: Option<&Metadata>) -> Result<()> {
        let Some(metadata) = metadata.filter(|metadata| metadata.offset > 0) else {
            return Ok(());
        };
        let etag = metadata
            .etag
            .as_deref()
            .expect("invariant: saved prefix has a strong ETag");
        if metadata.total == Some(metadata.offset) {
            return self.draft.headers.set(header::IF_NONE_MATCH, etag);
        }
        self.draft
            .headers
            .set(header::RANGE, format!("bytes={}-", metadata.offset))?;
        self.draft.headers.set(header::IF_RANGE, etag)
    }
}

async fn transfer(
    response: Response,
    partial: Arc<Partial>,
    metadata: Metadata,
    options: Box<Download>,
    caller: CallerWaits,
) -> Result<u64> {
    let cap = response.parts.compression.max_body_size as u64;
    let part = Arc::clone(&partial);
    let checksum = options.checksum.is_some();
    let mut digest = blocking(move || part.prefix(checksum)).await?;
    let mut written = partial.metadata().map_or(0, |metadata| metadata.offset);
    let mut body = response.bytes_stream()?;
    notify(&options, written, metadata.total);
    while let Some(chunk) = body.next().await {
        let remaining = cap.saturating_sub(written).min(
            metadata
                .total
                .map_or(u64::MAX, |total| total.saturating_sub(written)),
        );
        let chunk = append(Arc::clone(&partial), chunk, remaining).await?;
        if checksum {
            digest.update(&chunk);
        }
        written += chunk.len() as u64;
        notify(&options, written, metadata.total);
    }
    verify_length(written, metadata.total)?;
    verify_checksum(Arc::clone(&partial), digest, options.checksum).await?;
    let waiting = Arc::clone(&caller.0);
    blocking(move || partial.commit(&waiting)).await?;
    drop(caller);
    Ok(written)
}

async fn append(
    partial: Arc<Partial>,
    chunk: Result<bytes::Bytes>,
    remaining: u64,
) -> Result<bytes::Bytes> {
    let chunk = match chunk {
        Ok(chunk) => chunk,
        Err(error) => {
            blocking(move || partial.checkpoint()).await?;
            return Err(error);
        }
    };
    if chunk.len() as u64 > remaining {
        return Err(limit());
    }
    blocking(move || {
        partial.write(&chunk)?;
        Ok(chunk)
    })
    .await
}

fn verify_length(written: u64, total: Option<u64>) -> Result<()> {
    if total.is_some_and(|total| total != written) {
        return Err(Error::new(Kind::Body)
            .with_message("download length does not match the representation"));
    }
    Ok(())
}

async fn verify_checksum(
    partial: Arc<Partial>,
    digest: Sha256,
    expected: Option<[u8; 32]>,
) -> Result<()> {
    if let Some(expected) = expected
        && <[u8; 32]>::from(digest.finalize()) != expected
    {
        blocking(move || partial.discard()).await?;
        return Err(Error::new(Kind::Body).with_message("download SHA-256 mismatch"));
    }
    Ok(())
}

fn metadata(
    response: &Response,
    key: [u8; 32],
    old: Option<&Metadata>,
) -> Result<(Metadata, bool)> {
    if response
        .header(header::CONTENT_ENCODING.as_str())
        .is_some_and(|coding| !coding.eq_ignore_ascii_case("identity"))
    {
        return Err(Error::new(Kind::Body)
            .with_message("resumable download requires identity content encoding"));
    }
    let etag = response
        .header(header::ETAG.as_str())
        .filter(|etag| strong_etag(etag))
        .map(str::to_owned);
    let (total, append) = match response.status() {
        StatusCode::OK if !response.headers().contains_key(header::CONTENT_RANGE) => {
            (response.content_length(), false)
        }
        StatusCode::NOT_MODIFIED => {
            let old = old
                .filter(|old| old.offset > 0 && old.total == Some(old.offset))
                .ok_or_else(bad_range)?;
            if old.url != response.url().as_str() || etag != old.etag {
                return Err(bad_range());
            }
            (old.total, true)
        }
        StatusCode::PARTIAL_CONTENT => {
            let old = old.filter(|old| old.offset > 0).ok_or_else(bad_range)?;
            if old.url != response.url().as_str() || etag != old.etag {
                return Err(bad_range());
            }
            (Some(remainder(response, old)?), true)
        }
        _ => {
            response.error_for_status()?;
            return Err(bad_range());
        }
    };
    Ok((
        Metadata {
            version: FileVersion::default(),
            request: key,
            url: response.url().to_string(),
            etag,
            total,
            offset: 0,
        },
        append,
    ))
}

fn remainder(response: &Response, old: &Metadata) -> Result<u64> {
    let total = range(
        response
            .header(header::CONTENT_RANGE.as_str())
            .ok_or_else(bad_range)?,
        old.offset,
    )
    .ok_or_else(bad_range)?;
    if old.total.is_some_and(|previous| previous != total) {
        return Err(bad_range());
    }
    Ok(total)
}

fn range(value: &str, offset: u64) -> Option<u64> {
    let (span, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = span.split_once('-')?;
    if [start, end, total]
        .into_iter()
        .any(|number| !number.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    let (start, end, total) = (
        start.parse::<u64>().ok()?,
        end.parse::<u64>().ok()?,
        total.parse::<u64>().ok()?,
    );
    (start == offset && start <= end && end.checked_add(1)? == total).then_some(total)
}

fn strong_etag(value: &str) -> bool {
    value.len() >= 2
        && value.starts_with('"')
        && value.ends_with('"')
        && value.as_bytes()[1..value.len() - 1]
            .iter()
            .all(|byte| *byte == 0x21 || (0x23..=0x7e).contains(byte) || *byte >= 0x80)
}

fn notify(options: &Download, written: u64, total: Option<u64>) {
    if let Some(progress) = &options.progress {
        progress(written, total);
    }
}
fn bad_range() -> Error {
    Error::new(Kind::Body)
        .with_message("download response does not match the saved byte range and validator")
}
fn limit() -> Error {
    Error::new(Kind::BodyLimit)
        .with_message("download exceeds the complete representation size limit")
}

async fn blocking<T: Send + 'static>(
    run: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(run)
        .await
        .map_err(|error| Error::from(std::io::Error::other(error)))?
}
