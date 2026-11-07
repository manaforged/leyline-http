use std::path::Path;
use std::sync::Arc;

use futures_util::{FutureExt, StreamExt};
use http::{Method, StatusCode, header};
use sha2::{Digest, Sha256};

use super::{Draft, RequestBuilder};
use crate::core::atomic;
use crate::core::response::Response;
use crate::core::retry::RetryPlan;
use crate::error::{Error, Kind, Result};
use crate::profile::Encoding;

mod file;
#[cfg(test)]
#[path = "download_tests.rs"]
mod tests;
use file::{Metadata, Partial};

type Progress = Arc<dyn Fn(u64, Option<u64>) + Send + Sync>;

#[derive(Clone)]
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
    /// Downloads atomically, retries a dropped transfer under the retry policy, and resumes a saved prefix only when a strong ETag still matches.
    pub async fn download_to(mut self, path: impl AsRef<Path>) -> Result<u64> {
        let key = self.prepare_download()?;
        let options = self.draft.download.take().unwrap_or_default();
        let target = path.as_ref().to_path_buf();
        let retry = self.draft.carry.retry_policy.clone();
        let mut attempt = 0;
        loop {
            let result = self
                .next_transfer()
                .transfer_to(target.clone(), key, options.clone())
                .await;
            let Err(error) = result else {
                return result;
            };
            let failed = Err(error);
            match retry.plan(&failed, attempt) {
                RetryPlan::Backoff(wait) => tokio::time::sleep(wait).await,
                _ => return failed.map(|_| 0),
            }
            attempt += 1;
        }
    }

    fn next_transfer(&self) -> Self {
        let draft = &self.draft;
        Self {
            session: self.session.clone(),
            draft: Box::new(Draft {
                method: draft.method.clone(),
                url: draft.url.clone(),
                body: crate::wire::Body::default(),
                headers: draft.headers.clone(),
                query_params: Vec::new(),
                builder_error: None,
                compress: None,
                trusted_origin: draft.trusted_origin.clone(),
                deadline: draft.deadline,
                response: draft.response,
                download: None,
                carry: draft.carry.clone(),
            }),
        }
    }

    async fn transfer_to(
        mut self,
        target: std::path::PathBuf,
        key: [u8; 32],
        options: Box<Download>,
    ) -> Result<u64> {
        let resume = options.resume;
        let partial =
            Arc::new(atomic::blocking(move |_| Partial::open(target, key, resume)).await?);
        let old = partial.metadata();
        let offset = old.as_ref().map_or(0, |metadata| metadata.offset);
        if offset > self.draft.carry.body_rules.max_body_size as u64 {
            return Err(limit());
        }
        self.resume_headers(old.as_ref())?;
        let part = Arc::clone(&partial);
        let checksum = options.checksum.is_some() && offset > 0;
        let hashing = async move {
            if checksum {
                atomic::blocking(move |_| part.prefix()).await
            } else {
                Ok(Sha256::new())
            }
        };
        let (response, digest) = tokio::try_join!(self.send(), hashing)?;
        let (metadata, append) = metadata(&response, key, old.as_ref())?;
        if metadata
            .total
            .is_some_and(|total| total > response.parts.compression.max_body_size as u64)
        {
            return Err(limit());
        }
        let copy = metadata.clone();
        let part = Arc::clone(&partial);
        atomic::blocking(move |_| part.begin(copy, append)).await?;
        let digest = if append { digest } else { Sha256::new() };
        transfer(response, partial, (metadata, digest), options).await
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
            .set(header::ACCEPT_ENCODING, Encoding::IDENTITY)?;
        self.draft.carry.body_rules.decode = false;
        self.draft.carry.stream_response = true;
        self.prepare_in_place()?;
        let url = self
            .draft
            .url()
            .expect("invariant: download prepares its URL");
        self.session.download_key(url, &self.draft.headers)
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

const WRITE_BATCH: usize = 64;

async fn transfer(
    response: Response,
    partial: Arc<Partial>,
    (metadata, mut digest): (Metadata, Sha256),
    options: Box<Download>,
) -> Result<u64> {
    let cap = response.parts.compression.max_body_size as u64;
    let checksum = options.checksum.is_some();
    let mut written = partial.metadata().map_or(0, |metadata| metadata.offset);
    let mut body = response.bytes_stream()?;
    notify(&options, written, metadata.total);
    let complete = metadata.total == Some(written);
    while !complete && let Some(first) = body.next().await {
        let mut batch = vec![first];
        while batch.len() < WRITE_BATCH
            && let Some(Some(next)) = body.next().now_or_never()
        {
            batch.push(next);
        }
        let remaining = cap.saturating_sub(written).min(
            metadata
                .total
                .map_or(u64::MAX, |total| total.saturating_sub(written)),
        );
        for chunk in append(Arc::clone(&partial), batch, remaining).await? {
            if checksum {
                digest.update(&chunk);
            }
            written += chunk.len() as u64;
            notify(&options, written, metadata.total);
        }
    }
    verify_length(written, metadata.total)?;
    verify_checksum(Arc::clone(&partial), digest, options.checksum).await?;
    atomic::blocking(move |waiting| partial.commit(waiting)).await?;
    Ok(written)
}

async fn append(
    partial: Arc<Partial>,
    batch: Vec<Result<bytes::Bytes>>,
    remaining: u64,
) -> Result<Vec<bytes::Bytes>> {
    let mut chunks = Vec::with_capacity(batch.len());
    let mut failure = None;
    for chunk in batch {
        match chunk {
            Ok(chunk) => chunks.push(chunk),
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    if chunks.iter().map(|chunk| chunk.len() as u64).sum::<u64>() > remaining {
        return Err(limit());
    }
    let failed = failure.is_some();
    let chunks = atomic::blocking(move |_| {
        for chunk in &chunks {
            partial.write(chunk)?;
        }
        if failed {
            partial.checkpoint()?;
        }
        Ok(chunks)
    })
    .await?;
    failure.map_or(Ok(chunks), Err)
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
        atomic::blocking(move |_| partial.discard()).await?;
        return Err(Error::new(Kind::Body).with_message("download SHA-256 mismatch"));
    }
    Ok(())
}

fn metadata(
    response: &Response,
    key: [u8; 32],
    old: Option<&Metadata>,
) -> Result<(Metadata, bool)> {
    if !response.content_coding().is_identity() {
        return Err(Error::new(Kind::Body)
            .with_message("resumable download requires identity content encoding"));
    }
    let mut etag = response
        .header(header::ETAG.as_str())
        .filter(|etag| strong_etag(etag))
        .map(str::to_owned);
    let (total, append) = match response.status() {
        StatusCode::OK if !response.headers().contains_key(header::CONTENT_RANGE) => {
            (response.content_length(), false)
        }
        StatusCode::NOT_MODIFIED => {
            let old = saved(old, response, &etag, |old| old.total == Some(old.offset))?;
            (old.total, true)
        }
        StatusCode::PARTIAL_CONTENT => {
            let old = saved(old, response, &etag, |_| true)?;
            (Some(remainder(response, old)?), true)
        }
        StatusCode::RANGE_NOT_SATISFIABLE if let Some(old) = complete_prefix(old, response) => {
            etag.clone_from(&old.etag);
            (Some(old.offset), true)
        }
        _ => {
            response.error_for_status()?;
            return Err(bad_range());
        }
    };
    Ok((
        Metadata {
            version: file::MetadataVersion::default(),
            request: key,
            url: response.url().to_string(),
            etag,
            total,
            offset: 0,
        },
        append,
    ))
}

fn saved<'a>(
    old: Option<&'a Metadata>,
    response: &Response,
    etag: &Option<String>,
    accept: impl Fn(&Metadata) -> bool,
) -> Result<&'a Metadata> {
    old.filter(|old| {
        old.offset > 0 && accept(old) && old.url == response.url().as_str() && *etag == old.etag
    })
    .ok_or_else(bad_range)
}

fn complete_prefix<'a>(old: Option<&'a Metadata>, response: &Response) -> Option<&'a Metadata> {
    old.filter(|old| {
        old.offset > 0
            && old.total.is_none()
            && old.url == response.url().as_str()
            && unsatisfied_total(response) == Some(old.offset)
    })
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

fn unsatisfied_total(response: &Response) -> Option<u64> {
    let total = response
        .header(header::CONTENT_RANGE.as_str())?
        .strip_prefix("bytes */")?;
    if total.is_empty() || !total.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    total.parse().ok()
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
