use super::*;
use crate::core::BoxedStream;

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::trace;

mod body;
#[cfg(feature = "bench-internals")]
pub mod floor;
mod head;
mod upload;

use upload::{Sent, Upload};

pub(super) use head::validate;

enum Framing {
    None,
    Buffered(Bytes),
    Fixed { stream: BoxedStream },
    Chunked { stream: BoxedStream },
}

async fn send_h1_request(
    stream: &mut dyn H1Io,
    method: &str,
    url: &url::Url,
    headers: &[HeaderPair],
    body: Body,
    target: H1Target,
    mut buf: Vec<u8>,
) -> Result<(bool, Upload), Error> {
    let (request_target, authority) = head::target(url, target)?;
    let (framing, length) = head::frame(method, headers, body);
    head::Head {
        method,
        target: &request_target,
        host: (!contains_header(headers, "host")).then_some(&*authority),
        headers,
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
    method: &str,
    sent: Upload,
) -> Result<(H1Head, bool), Error> {
    match sent.sent {
        Sent::Complete => Ok((read_h1_head(stream, method, sent.received).await?, true)),
        Sent::Answered => Ok((read_h1_head(stream, method, sent.received).await?, false)),
        Sent::Broken(write) => match read_h1_head(stream, method, sent.received).await {
            Ok(head) => Ok((head, false)),
            Err(read) => {
                drop(read);
                Err(Error::from(write))
            }
        },
    }
}
pub(super) fn compute_reusable(
    client_asked_close: bool,
    resp_headers: &HeaderMap,
    minor: u8,
) -> bool {
    let connection = text(resp_headers, http::header::CONNECTION);
    let server_says_close = connection.is_some_and(|v| has_token(v, "close"));
    let server_says_keepalive = connection.is_some_and(|v| has_token(v, "keep-alive"));
    if client_asked_close || server_says_close {
        false
    } else if minor >= 1 {
        true
    } else {
        server_says_keepalive
    }
}
pub(super) async fn exchange_head_on_stream(
    stream: &mut dyn H1Io,
    method: &str,
    url: &url::Url,
    headers: &[HeaderPair],
    body: Body,
    target: H1Target,
    buf: Vec<u8>,
) -> Result<(H1Head, bool), Error> {
    let host = url.host_str().unwrap_or("");
    let started = trace::on().then(Instant::now);
    let (client_asked_close, sent) =
        send_h1_request(stream, method, url, headers, body, target, buf).await?;
    trace::sent(
        host,
        method,
        crate::util::request_target(url),
        http::Version::HTTP_11,
        started.map_or(Duration::ZERO, |started| started.elapsed()),
    );
    let started = trace::on().then(Instant::now);
    let (head, complete) = response_head(stream, method, sent).await?;
    trace::head(
        host,
        head.status,
        http::Version::HTTP_11,
        started.map_or(Duration::ZERO, |started| started.elapsed()),
        &head.headers,
    );
    let reusable = compute_reusable(client_asked_close, &head.headers, head.minor)
        && !matches!(head.framing, BodyFraming::ToClose)
        && complete;
    Ok((head, reusable))
}
pub(super) fn method_typically_has_body(method: &str) -> bool {
    ["POST", "PUT", "PATCH"]
        .iter()
        .any(|m| method.eq_ignore_ascii_case(m))
}
#[cfg(feature = "websocket")]
pub(crate) async fn upgrade_on_stream(
    stream: &mut dyn H1Io,
    url: &url::Url,
    headers: &[HeaderPair],
) -> Result<((u16, HeaderMap, u8), Vec<u8>), Error> {
    head::validate("GET", headers)?;
    let (_, sent) = send_h1_request(
        stream,
        "GET",
        url,
        headers,
        Body::default(),
        H1Target::OriginForm,
        Vec::new(),
    )
    .await?;
    let mut buf = sent.received;
    let header_end = read_h1_headers(stream, &mut buf).await?;
    let head = parse_h1_head(&buf[..header_end])?;
    buf.drain(..header_end + 4);
    Ok((head, buf))
}
