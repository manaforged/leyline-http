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

use crate::core::BodyRules;
use crate::core::error::{Error, Kind, Result};

const MAX_CODINGS: usize = 4;

use http::HeaderMap;

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
            return Err(Error::new(Kind::Decode).with_message(format!(
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
    headers: HeaderMap,
    config: &BodyRules,
) -> Result<(Vec<u8>, HeaderMap)> {
    let (body, decoded) = decompress_body(body, pair_codings(&headers).as_deref(), config)?;
    Ok((body, strip_decoded(headers, decoded)))
}

pub(crate) async fn buffer_and_strip(
    bs: crate::core::body_stream::BodyStream,
    headers: HeaderMap,
    config: &BodyRules,
) -> Result<(Vec<u8>, HeaderMap)> {
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
        let chunk = chunk.map_err(Error::from)?;
        read += chunk.len();
        if read > config.max_body_size {
            return Err(BodyLimit::session(config.max_body_size).error());
        }
        decoder.feed(&chunk, &mut out)?;
    }
    decoder.finish(&mut out)?;
    Ok((out, true))
}

fn pair_codings(headers: &HeaderMap) -> Option<String> {
    content_codings(headers.get_all(http::header::CONTENT_ENCODING).iter())
}

fn strip_decoded(mut headers: HeaderMap, decoded: bool) -> HeaderMap {
    if decoded {
        headers.remove(http::header::CONTENT_ENCODING);
        headers.remove(http::header::CONTENT_LENGTH);
    }
    headers
}

async fn drain_stream_into_vec(
    mut bs: crate::core::body_stream::BodyStream,
    limit: usize,
) -> Result<Vec<u8>> {
    use futures_util::StreamExt;
    let mut out = Vec::new();
    while let Some(chunk) = bs.next().await {
        let chunk = chunk.map_err(Error::from)?;
        if out.len() + chunk.len() > limit {
            return Err(BodyLimit::session(limit).error());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
