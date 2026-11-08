use super::*;
use crate::wire::BoxedStream;

use std::collections::VecDeque;

use crate::trace;

mod body;
#[cfg(feature = "bench-internals")]
pub mod floor;
mod head;
mod upload;

use upload::{Sent, Upload};

#[cfg(feature = "bench-internals")]
pub(super) use head::invalid_method;
pub(super) use head::validate;

#[derive(Clone, Copy)]
pub(super) struct Fields<'a> {
    pub(super) pairs: &'a [HeaderPair],
    pub(super) lines: Option<&'a [u8]>,
}

enum Framing {
    None,
    Buffered(Bytes),
    Fixed { stream: BoxedStream },
    Chunked { stream: BoxedStream },
}

async fn send_h1_request(
    stream: &mut dyn H1Io,
    method: &Method,
    url: &url::Url,
    fields: Fields<'_>,
    body: Body,
    target: H1Target,
    mut buf: Vec<u8>,
) -> Result<(bool, Upload), H1Error> {
    let Fields {
        pairs: headers,
        lines,
    } = fields;
    let (request_target, authority) = head::target(url, target)?;
    let (framing, length) = head::frame(method, headers, body);
    head::Head {
        method: method.as_str(),
        target: &request_target,
        host: (!contains_header(headers, "host")).then_some(&*authority),
        headers,
        lines,
        length: length.as_ref().map(|(name, value)| (*name, value.as_ref())),
        keep_alive: !contains_header(headers, "connection"),
    }
    .encode(&mut buf);
    stream.write_all(&buf).await?;

    let sent = match framing {
        Framing::None => {
            stream.flush().await?;
            Upload::idle(buf)
        }
        Framing::Buffered(b) => upload::upload(stream, VecDeque::from([b]), None, buf).await?,
        Framing::Fixed { stream: chunks } => {
            upload::upload(
                stream,
                VecDeque::new(),
                Some((chunks, body::Encoder::Fixed)),
                buf,
            )
            .await?
        }
        Framing::Chunked { stream: chunks } => {
            let encoder = body::Encoder::Chunked;
            upload::upload(stream, VecDeque::new(), Some((chunks, encoder)), buf).await?
        }
    };

    Ok((header_contains_token(headers, "connection", "close"), sent))
}
async fn response_head(
    stream: &mut dyn H1Io,
    method: &Method,
    sent: Upload,
) -> Result<(H1Head, bool), H1Error> {
    match sent.sent {
        Sent::Complete => Ok((read_h1_head(stream, method, sent.received).await?, true)),
        Sent::Answered => Ok((read_h1_head(stream, method, sent.received).await?, false)),
        Sent::Broken(write) => match read_h1_head(stream, method, sent.received).await {
            Ok(head) => Ok((head, false)),
            Err(read) => {
                drop(read);
                Err(H1Error::from(write))
            }
        },
    }
}
pub(super) fn compute_reusable(
    client_asked_close: bool,
    seen: crate::wire::Seen,
    minor: u8,
) -> bool {
    if client_asked_close || seen.close {
        false
    } else if minor >= 1 {
        true
    } else {
        seen.keep_alive
    }
}
pub(super) async fn exchange_head_on_stream(
    stream: &mut dyn H1Io,
    method: &Method,
    url: &url::Url,
    fields: Fields<'_>,
    body: Body,
    target: H1Target,
    buf: Vec<u8>,
) -> Result<(H1Head, bool), H1Error> {
    let host = url.host_str().unwrap_or("");
    let (client_asked_close, sent) =
        send_h1_request(stream, method, url, fields, body, target, buf).await?;
    let (head, complete) = response_head(stream, method, sent).await?;
    trace::head(host, head.status, http::Version::HTTP_11, &head.headers);
    let reusable = compute_reusable(client_asked_close, head.headers.seen(), head.minor)
        && !matches!(head.framing, BodyFraming::ToClose)
        && complete;
    Ok((head, reusable))
}
#[cfg(feature = "websocket")]
pub(crate) async fn upgrade_on_stream(
    stream: &mut dyn H1Io,
    url: &url::Url,
    headers: &[HeaderPair],
) -> Result<((u16, http::HeaderMap, u8), Vec<u8>), Error> {
    head::validate(&Method::GET, headers)?;
    let (_, sent) = send_h1_request(
        stream,
        &Method::GET,
        url,
        Fields {
            pairs: headers,
            lines: None,
        },
        Body::default(),
        H1Target::OriginForm,
        Vec::new(),
    )
    .await
    .map_err(H1Error::into_error)?;
    let mut buf = sent.received;
    let header_end = read_h1_headers(stream, &mut buf)
        .await
        .map_err(H1Error::into_error)?;
    let head = parse_h1_head(&buf[..header_end])?;
    buf.drain(..header_end + 4);
    Ok((head, buf))
}
