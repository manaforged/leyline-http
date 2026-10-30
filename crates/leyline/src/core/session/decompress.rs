mod limit;
#[cfg(any(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-deflate",
    feature = "compression-zstd"
))]
mod sink;
mod stage;

pub(crate) use limit::BodyLimit;
use limit::{Cap, Overflow};
use stage::Stage;

use super::helpers::header_values;
use crate::core::BodyRules;
use crate::core::error::{Cause, Error, Result};

const MAX_CODINGS: usize = 4;

type HeaderPairs = Vec<(http::HeaderName, http::HeaderValue)>;

pub(crate) struct Decoder {
    stages: Vec<Stage>,
    fed: bool,
    produced: usize,
    cap: Cap,
}

impl Decoder {
    pub(crate) fn new(encoding: Option<&str>, config: &BodyRules) -> Result<Option<Self>> {
        Self::with_cap(
            encoding,
            config,
            Cap::fail(BodyLimit::session(config.max_body_size)),
        )
    }

    pub(crate) fn capped(encoding: Option<&str>, config: &BodyRules) -> Result<Self> {
        let cap = Cap::fail(BodyLimit::session(config.max_body_size));
        Ok(Self::with_cap(encoding, config, cap)?.unwrap_or_else(|| Self::over(Vec::new(), cap)))
    }

    pub(crate) fn truncated(
        encoding: Option<&str>,
        config: &BodyRules,
        limit: usize,
    ) -> Result<Option<Self>> {
        let cap = Cap::truncate_at(BodyLimit::session(config.max_body_size), limit);
        Self::with_cap(encoding, config, cap)
    }

    fn with_cap(encoding: Option<&str>, config: &BodyRules, cap: Cap) -> Result<Option<Self>> {
        let Some(encoding) = encoding else {
            return Ok(None);
        };
        let encodings: Vec<&str> = encoding.split(',').map(str::trim).collect();
        if encodings.len() > MAX_CODINGS {
            return Err(Error::new(Cause::Decode).with_message(format!(
                "content-encoding lists {} codings; at most {MAX_CODINGS} are decoded",
                encodings.len()
            )));
        }
        if !encodings.iter().all(|enc| config.allows(enc)) {
            return Ok(None);
        }
        let stages = encodings
            .iter()
            .rev()
            .copied()
            .map(|enc| Stage::new(enc, cap))
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(Self::over(stages, cap)))
    }

    fn over(stages: Vec<Stage>, cap: Cap) -> Self {
        Self {
            stages,
            fed: false,
            produced: 0,
            cap,
        }
    }

    pub(crate) fn feed(&mut self, chunk: &[u8], out: &mut Vec<u8>) -> Result<()> {
        if chunk.is_empty() {
            return Ok(());
        }
        self.fed = true;
        let mut data = chunk.to_vec();
        for stage in &mut self.stages {
            data = stage.write(&data)?;
        }
        self.emit(data, out)
    }

    pub(crate) fn finish(&mut self, out: &mut Vec<u8>) -> Result<()> {
        if !self.fed {
            return Ok(());
        }
        for index in 0..self.stages.len() {
            let mut data = self.stages[index].finish()?;
            for stage in &mut self.stages[index + 1..] {
                data = stage.write(&data)?;
            }
            self.emit(data, out)?;
        }
        Ok(())
    }

    fn emit(&mut self, data: Vec<u8>, out: &mut Vec<u8>) -> Result<()> {
        let room = self.cap.room(self.produced);
        self.produced = self.produced.saturating_add(data.len());
        if data.len() <= room {
            out.extend_from_slice(&data);
            return Ok(());
        }
        match self.cap.overflow {
            Overflow::Fail => Err(self.cap.limit.error()),
            Overflow::Truncate => {
                out.extend_from_slice(&data[..room]);
                Ok(())
            }
        }
    }
}

pub(crate) fn decompress_body(
    body: Vec<u8>,
    encoding: Option<&str>,
    config: &BodyRules,
) -> Result<(Vec<u8>, bool)> {
    let Some(mut decoder) = Decoder::new(encoding, config)? else {
        return Ok((body, false));
    };
    let mut out = Vec::new();
    decoder.feed(&body, &mut out)?;
    decoder.finish(&mut out)?;
    Ok((out, true))
}

pub(crate) fn content_codings<'a>(
    values: impl IntoIterator<Item = &'a http::HeaderValue>,
) -> Option<String> {
    let codings: Vec<String> = values
        .into_iter()
        .map(|v| {
            String::from_utf8_lossy(v.as_bytes())
                .trim()
                .to_ascii_lowercase()
        })
        .collect();
    (!codings.is_empty()).then(|| codings.join(", "))
}

pub(crate) fn decompress_and_strip(
    body: Vec<u8>,
    headers: HeaderPairs,
    config: &BodyRules,
) -> Result<(Vec<u8>, HeaderPairs)> {
    let (body, decoded) = decompress_body(body, pair_codings(&headers).as_deref(), config)?;
    Ok((body, strip_decoded(headers, decoded)))
}

pub(crate) async fn buffer_and_strip(
    bs: crate::core::body_stream::BodyStream,
    headers: HeaderPairs,
    config: &BodyRules,
) -> Result<(Vec<u8>, HeaderPairs)> {
    let (body, decoded) = buffer_decoded(bs, pair_codings(&headers).as_deref(), config).await?;
    Ok((body, strip_decoded(headers, decoded)))
}

pub(crate) async fn buffer_decoded(
    bs: crate::core::body_stream::BodyStream,
    encoding: Option<&str>,
    config: &BodyRules,
) -> Result<(Vec<u8>, bool)> {
    use futures_util::StreamExt;
    let Some(mut decoder) = Decoder::new(encoding, config)? else {
        return Ok((
            drain_stream_into_vec(bs, config.max_body_size).await?,
            false,
        ));
    };
    let mut bs = bs;
    let mut read = 0usize;
    let mut out = Vec::new();
    while let Some(chunk) = bs.next().await {
        let chunk = chunk.map_err(body_read_error)?;
        read += chunk.len();
        if read > config.max_body_size {
            return Err(BodyLimit::session(config.max_body_size).error());
        }
        decoder.feed(&chunk, &mut out)?;
    }
    decoder.finish(&mut out)?;
    Ok((out, true))
}

fn pair_codings(headers: &[(http::HeaderName, http::HeaderValue)]) -> Option<String> {
    content_codings(header_values(headers, "content-encoding"))
}

fn strip_decoded(headers: HeaderPairs, decoded: bool) -> HeaderPairs {
    if decoded {
        headers
            .into_iter()
            .filter(|(k, _)| *k != "content-encoding" && *k != "content-length")
            .collect()
    } else {
        headers
    }
}

async fn drain_stream_into_vec(
    mut bs: crate::core::body_stream::BodyStream,
    limit: usize,
) -> Result<Vec<u8>> {
    use futures_util::StreamExt;
    let mut out = Vec::new();
    while let Some(chunk) = bs.next().await {
        let chunk = chunk.map_err(body_read_error)?;
        if out.len() + chunk.len() > limit {
            return Err(BodyLimit::session(limit).error());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

pub(crate) fn body_read_error(e: std::io::Error) -> Error {
    if let Some(limit) = BodyLimit::of_io(&e) {
        return limit.error();
    }
    if e.get_ref().is_some_and(|inner| inner.is::<Error>()) {
        if let Some(inner) = e.into_inner()
            && let Ok(err) = inner.downcast::<Error>()
        {
            return *err;
        }
        return Error::new(Cause::Body).with_message("response body stream failed");
    }
    if e.kind() == std::io::ErrorKind::TimedOut {
        Error::new(Cause::Timeout).with_source(e)
    } else {
        Error::from(e)
    }
}

#[cfg(test)]
mod tests;
