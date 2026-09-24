use super::*;

pub(super) async fn read_chunk_trailers<S>(
    stream: &mut S,
    buf: &mut Vec<u8>,
) -> Result<(), H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    loop {
        let line_end = read_until_crlf(stream, buf).await?;
        let empty = line_end == 0;
        buf.drain(..line_end + 2);
        if empty {
            return Ok(());
        }
    }
}

pub(super) const MAX_H1_CHUNK_LINE_BYTES: usize = 16 * 1024;

pub(super) async fn read_until_crlf<S>(
    stream: &mut S,
    buf: &mut Vec<u8>,
) -> Result<usize, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    loop {
        if let Some(pos) = buf.windows(2).position(|w| w == b"\r\n") {
            return Ok(pos);
        }
        if buf.len() > MAX_H1_CHUNK_LINE_BYTES {
            return Err(H1PooledError::Http(format!(
                "chunked size/trailer line exceeds {MAX_H1_CHUNK_LINE_BYTES} bytes"
            )));
        }
        read_more(stream, buf).await?;
    }
}

pub(super) async fn read_until_available<S>(
    stream: &mut S,
    buf: &mut Vec<u8>,
    len: usize,
) -> Result<(), H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    while buf.len() < len {
        read_more(stream, buf).await?;
    }
    Ok(())
}

pub(super) async fn read_more<S>(stream: &mut S, buf: &mut Vec<u8>) -> Result<(), H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut tmp = [0u8; 8192];
    let n = stream.read(&mut tmp).await?;
    if n == 0 {
        return Err(H1PooledError::ConnectionClosed(
            "during chunked body".into(),
        ));
    }
    buf.extend_from_slice(&tmp[..n]);
    if buf.len() > MAX_H1_BODY_BYTES {
        return Err(H1PooledError::Http(format!(
            "HTTP/1.1 body exceeds {MAX_H1_BODY_BYTES} bytes"
        )));
    }
    Ok(())
}
