//! Body write phases for a streamed HTTP/1.1 request.

use super::*;

/// Write a streamed body that must match the declared content-length exactly.
pub(super) async fn fixed(
    stream: &mut dyn H1Io,
    mut body: BodyStream,
    length: u64,
) -> Result<(), H1PooledError> {
    let mut sent: u64 = 0;
    while let Some(chunk) = body.next().await {
        let chunk: Bytes = chunk?;
        if sent + chunk.len() as u64 > length {
            return Err(H1PooledError::Http(
                "streaming body exceeded declared content-length".into(),
            ));
        }
        stream.write_all(&chunk).await?;
        sent += chunk.len() as u64;
    }
    if sent != length {
        return Err(H1PooledError::Http(format!(
            "streaming body ended before declared content-length ({sent}/{length})"
        )));
    }
    Ok(())
}

/// Write a streamed body as chunked transfer-coding, then the terminating chunk.
pub(super) async fn chunked(
    stream: &mut dyn H1Io,
    mut body: BodyStream,
) -> Result<(), H1PooledError> {
    while let Some(chunk) = body.next().await {
        let chunk: Bytes = chunk?;
        if chunk.is_empty() {
            continue;
        }
        let hdr = format!("{:X}\r\n", chunk.len());
        stream.write_all(hdr.as_bytes()).await?;
        stream.write_all(&chunk).await?;
        stream.write_all(b"\r\n").await?;
    }
    stream.write_all(b"0\r\n\r\n").await?;
    Ok(())
}
