mod limit;
#[cfg(leyline_any_codec)]
mod sink;
mod stage;

use crate::error::BodyLimit;
use limit::{Cap, Overflow};
use stage::Stage;

use crate::core::BodyRules;
use crate::core::config::{Codings, MAX_CODINGS};
use crate::error::{Error, Kind, Result};
use crate::profile::Encoding;

use http::HeaderMap;

pub(crate) struct Decoder {
    stages: Vec<Stage>,
    fed: bool,
    produced: usize,
    cap: Cap,
}

impl Decoder {
    pub(crate) fn new(codings: &Codings, config: &BodyRules) -> Result<Option<Self>> {
        Self::with_cap(
            codings,
            config,
            Cap::fail(BodyLimit::session(config.max_body_size)),
        )
    }

    pub(crate) fn capped(codings: &Codings, config: &BodyRules) -> Result<Self> {
        let cap = Cap::fail(BodyLimit::session(config.max_body_size));
        Ok(Self::with_cap(codings, config, cap)?.unwrap_or_else(|| Self::over(Vec::new(), cap)))
    }

    pub(crate) fn truncated(
        codings: &Codings,
        config: &BodyRules,
        limit: usize,
    ) -> Result<Option<Self>> {
        let cap = Cap::truncate_at(BodyLimit::session(config.max_body_size), limit);
        Self::with_cap(codings, config, cap)
    }

    fn with_cap(codings: &Codings, config: &BodyRules, cap: Cap) -> Result<Option<Self>> {
        match codings {
            Codings::Absent | Codings::Unknown => Ok(None),
            Codings::TooMany(count) => Err(Error::new(Kind::Decode).with_message(format!(
                "content-encoding lists {count} codings; at most {MAX_CODINGS} are decoded"
            ))),
            Codings::Listed(listed) => Self::stages(listed, config, cap),
        }
    }

    fn stages(listed: &[Encoding], config: &BodyRules, cap: Cap) -> Result<Option<Self>> {
        if !listed.iter().all(|&coding| config.allows(coding)) {
            return Ok(None);
        }
        let stages = listed
            .iter()
            .rev()
            .map(|&coding| Stage::new(coding, cap))
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
    codings: &Codings,
    config: &BodyRules,
) -> Result<(Vec<u8>, bool)> {
    let Some(mut decoder) = Decoder::new(codings, config)? else {
        return Ok((body, false));
    };
    let mut out = Vec::new();
    decoder.feed(&body, &mut out)?;
    decoder.finish(&mut out)?;
    Ok((out, true))
}

pub(crate) fn decompress_and_strip(
    body: Vec<u8>,
    mut headers: HeaderMap,
    config: &BodyRules,
) -> Result<(Vec<u8>, HeaderMap)> {
    let (body, decoded) = decompress_body(body, &pair_codings(&headers), config)?;
    if decoded {
        strip_decoded(&mut headers);
    }
    Ok((body, headers))
}

pub(crate) async fn buffer_and_strip(
    bs: crate::core::body_stream::BodyStream,
    mut headers: HeaderMap,
    config: &BodyRules,
) -> Result<(Vec<u8>, HeaderMap)> {
    let (body, decoded) = buffer_decoded(bs, &pair_codings(&headers), config).await?;
    if decoded {
        strip_decoded(&mut headers);
    }
    Ok((body, headers))
}

pub(crate) async fn buffer_decoded(
    bs: crate::core::body_stream::BodyStream,
    codings: &Codings,
    config: &BodyRules,
) -> Result<(Vec<u8>, bool)> {
    use futures_util::StreamExt;
    let Some(mut decoder) = Decoder::new(codings, config)? else {
        return Ok((
            drain_stream_into_vec(bs, BodyLimit::session(config.max_body_size)).await?,
            false,
        ));
    };
    let mut bs = bs;
    let limit = BodyLimit::session(config.max_body_size);
    let mut read = 0usize;
    let mut out = Vec::new();
    while let Some(chunk) = bs.next().await {
        let chunk = chunk.map_err(Error::from)?;
        limit.admit(read, chunk.len()).map_err(BodyLimit::error)?;
        read += chunk.len();
        decoder.feed(&chunk, &mut out)?;
    }
    decoder.finish(&mut out)?;
    Ok((out, true))
}

fn pair_codings(headers: &HeaderMap) -> Codings {
    Codings::of(headers.get_all(http::header::CONTENT_ENCODING).iter())
}

pub(crate) static DECODED_HEADERS: [http::HeaderName; 2] =
    [http::header::CONTENT_ENCODING, http::header::CONTENT_LENGTH];

pub(crate) fn strip_decoded(headers: &mut HeaderMap) {
    for name in &DECODED_HEADERS {
        headers.remove(name);
    }
}

async fn drain_stream_into_vec(
    mut bs: crate::core::body_stream::BodyStream,
    limit: BodyLimit,
) -> Result<Vec<u8>> {
    use futures_util::StreamExt;
    let mut out = Vec::new();
    while let Some(chunk) = bs.next().await {
        let chunk = chunk.map_err(Error::from)?;
        limit
            .admit(out.len(), chunk.len())
            .map_err(BodyLimit::error)?;
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
