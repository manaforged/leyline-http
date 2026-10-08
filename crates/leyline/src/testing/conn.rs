use std::io;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::types::{
    CLOSE, CONNECTION, ChunkedBody, Handler, RecordedRequest, Recorder, TestResponse,
};
use crate::pool::H1PooledError;
use crate::pool::h1::MAX_H1_HEADER_BYTES;
use crate::pool::h1::parse::{parse_h1_head, read_chunked_body};

const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;
const HEAD_END: &[u8] = b"\r\n\r\n";
const READ_CHUNK: usize = 8192;

pub(super) async fn serve<S>(mut stream: S, handler: Handler, recorder: Recorder)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut buf = Vec::new();
    while let Ok(Some(request)) = read_request(&mut stream, &mut buf).await {
        let close_requested = request
            .header(CONNECTION)
            .is_some_and(|value| value.eq_ignore_ascii_case(CLOSE));
        let head_only = request.method.eq_ignore_ascii_case("HEAD");
        if !recorder.record(request.clone()) {
            return;
        }
        let Some(response) = respond(&handler, &request).await else {
            return;
        };
        let close = close_requested || response.closes();
        if write_response(&mut stream, &response, head_only)
            .await
            .is_err()
            || close
        {
            drop(stream.shutdown().await);
            return;
        }
    }
}

async fn respond(handler: &Handler, request: &RecordedRequest) -> Option<TestResponse> {
    let handler = Arc::clone(handler);
    let request = request.clone();
    let response = tokio::task::spawn_blocking(move || handler(&request))
        .await
        .ok()?;
    tokio::time::sleep(response.delay).await;
    Some(response)
}

async fn read_request<S>(stream: &mut S, buf: &mut Vec<u8>) -> io::Result<Option<RecordedRequest>>
where
    S: AsyncRead + Unpin,
{
    let Some(end) = read_head(stream, buf).await? else {
        return Ok(None);
    };
    let raw: Vec<u8> = buf.drain(..end + HEAD_END.len()).collect();
    let head = String::from_utf8_lossy(&raw[..end]).into_owned();
    let (request_line, header_block) = head.split_once("\r\n").unwrap_or((head.as_str(), ""));
    let mut parts = request_line.split(' ');
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Err(invalid(format!("bad request line: {request_line}")));
    };
    let (_, headers, _) =
        parse_h1_head(&format!("HTTP/1.1 200 OK\r\n{header_block}")).map_err(to_io)?;
    let mut request = RecordedRequest {
        method: method.to_owned(),
        target: target.to_owned(),
        headers,
        body: Vec::new(),
        request_line: request_line.to_owned(),
        raw,
    };
    request.body = read_body(stream, buf, &request).await?;
    Ok(Some(request))
}

async fn read_head<S>(stream: &mut S, buf: &mut Vec<u8>) -> io::Result<Option<usize>>
where
    S: AsyncRead + Unpin,
{
    let mut chunk = [0u8; READ_CHUNK];
    loop {
        if let Some(end) = buf.windows(HEAD_END.len()).position(|w| w == HEAD_END) {
            return Ok(Some(end));
        }
        if buf.len() > MAX_H1_HEADER_BYTES {
            return Err(invalid(format!(
                "request head exceeds {MAX_H1_HEADER_BYTES} bytes"
            )));
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..read]);
    }
}

async fn read_body<S>(
    stream: &mut S,
    buf: &mut Vec<u8>,
    request: &RecordedRequest,
) -> io::Result<Vec<u8>>
where
    S: AsyncRead + Unpin,
{
    let chunked = request
        .header("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"));
    if chunked {
        let initial = std::mem::take(buf);
        return read_chunked_body(stream, initial, MAX_BODY_BYTES)
            .await
            .map_err(to_io);
    }
    let length = match request.header("content-length") {
        Some(value) => value
            .trim()
            .parse::<usize>()
            .map_err(|err| invalid(format!("bad content-length: {err}")))?,
        None => 0,
    };
    if length > MAX_BODY_BYTES {
        return Err(invalid(format!(
            "request body exceeds {MAX_BODY_BYTES} bytes"
        )));
    }
    let mut chunk = [0u8; READ_CHUNK];
    while buf.len() < length {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        buf.extend_from_slice(&chunk[..read]);
    }
    Ok(buf.drain(..length).collect())
}

async fn write_response<S>(
    stream: &mut S,
    response: &TestResponse,
    head_only: bool,
) -> io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    stream.write_all(head(response).as_bytes()).await?;
    if !head_only {
        match &response.chunks {
            Some(chunked) => write_chunks(stream, chunked).await?,
            None => stream.write_all(&response.body).await?,
        }
    }
    stream.flush().await
}

fn head(response: &TestResponse) -> String {
    let reason = http::StatusCode::from_u16(response.status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or("");
    let mut head = format!("HTTP/1.1 {} {reason}\r\n", response.status);
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    let framed = response.headers.iter().any(|(name, _)| {
        name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("transfer-encoding")
    });
    if !framed {
        match response.chunks {
            Some(_) => head.push_str("transfer-encoding: chunked\r\n"),
            None => head.push_str(&format!("content-length: {}\r\n", response.body.len())),
        }
    }
    head.push_str("\r\n");
    head
}

async fn write_chunks<S>(stream: &mut S, chunked: &ChunkedBody) -> io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    for (index, part) in chunked
        .parts
        .iter()
        .filter(|part| !part.is_empty())
        .enumerate()
    {
        if index > 0 {
            tokio::time::sleep(chunked.pause).await;
        }
        stream
            .write_all(format!("{:x}\r\n", part.len()).as_bytes())
            .await?;
        stream.write_all(part).await?;
        stream.write_all(b"\r\n").await?;
        stream.flush().await?;
    }
    stream.write_all(b"0\r\n\r\n").await
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn to_io(err: H1PooledError) -> io::Error {
    invalid(err.to_string())
}
