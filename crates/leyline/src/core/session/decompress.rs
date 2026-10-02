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
use stage::Stage;

use crate::core::CompressionConfig;
use crate::core::error::{Error, Kind, Result};

const MAX_CODINGS: usize = 4;

type HeaderPairs = Vec<(http::HeaderName, http::HeaderValue)>;

pub(crate) struct Decoder {
    stages: Vec<Stage>,
    fed: bool,
    produced: usize,
    limit: BodyLimit,
}

impl Decoder {
    pub(crate) fn new(encoding: Option<&str>, config: &CompressionConfig) -> Result<Option<Self>> {
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
            .map(|enc| Stage::new(enc, config.max_body_size))
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(Self {
            stages,
            fed: false,
            produced: 0,
            limit: BodyLimit::session(config.max_body_size),
        }))
    }

    pub(crate) fn capped(
        encoding: Option<&str>,
        config: &CompressionConfig,
        limit: Option<u64>,
    ) -> Result<Self> {
        let mut decoder = Self::new(encoding, config)?.unwrap_or(Self {
            stages: Vec::new(),
            fed: false,
            produced: 0,
            limit: BodyLimit::session(config.max_body_size),
        });
        decoder.limit = decoder.limit.tighter(limit);
        Ok(decoder)
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
        self.produced += data.len();
        if self.produced > self.limit.bytes {
            return Err(self.limit.error());
        }
        out.extend_from_slice(&data);
        Ok(())
    }
}

pub(crate) fn decompress_body(
    body: Vec<u8>,
    encoding: Option<&str>,
    config: &CompressionConfig,
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
    config: &CompressionConfig,
) -> Result<(Vec<u8>, HeaderPairs)> {
    let content_encoding = content_codings(
        headers
            .iter()
            .filter(|(k, _)| *k == "content-encoding")
            .map(|(_, v)| v),
    );
    let (body, decoded) = decompress_body(body, content_encoding.as_deref(), config)?;
    let headers = if decoded {
        headers
            .into_iter()
            .filter(|(k, _)| *k != "content-encoding" && *k != "content-length")
            .collect()
    } else {
        headers
    };
    Ok((body, headers))
}

pub(crate) async fn drain_stream_into_vec(
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
        return Error::new(Kind::Body).with_message("response body stream failed");
    }
    if e.kind() == std::io::ErrorKind::TimedOut {
        Error::new(Kind::Timeout).with_source(e)
    } else {
        Error::from(e)
    }
}

#[cfg(test)]
mod tests;
