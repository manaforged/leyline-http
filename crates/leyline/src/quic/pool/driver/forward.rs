use super::*;

pub(in crate::quic::pool) fn forward_stream_body(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    stream_id: u64,
    stream: &mut H3Stream,
    scratch: &mut [u8],
    max_response_body_bytes: u64,
) -> bool {
    use tokio::sync::mpsc::error::TrySendError;

    let Some(tx) = stream.stream_tx.clone() else {
        return false;
    };

    if let Some(chunk) = stream.stalled.take() {
        match tx.try_send(Ok(chunk)) {
            Ok(()) => {}
            Err(TrySendError::Full(item)) => {
                if let Ok(b) = item {
                    stream.stalled = Some(b);
                }
                return false;
            }
            Err(TrySendError::Closed(_)) => {
                shutdown(conn, stream_id, quiche::Shutdown::Read, 0);
                reset_upload_half(conn, stream_id, stream);
                return true;
            }
        }
    }

    let mut drained_clean = false;
    loop {
        match h3.recv_body(conn, stream_id, scratch) {
            Ok(0) => {
                drained_clean = true;
                break;
            }
            Ok(n) => {
                if let Err(new_len) =
                    check_body_budget(stream.body_bytes_seen, n, max_response_body_bytes)
                {
                    shutdown(
                        conn,
                        stream_id,
                        quiche::Shutdown::Read,
                        quiche::h3::WireErrorCode::ExcessiveLoad as u64,
                    );
                    deliver_stream_error(
                        &tx,
                        std::io::Error::other(format!(
                            "h3: response body exceeded max_response_body_bytes ({new_len} > {max_response_body_bytes})"
                        )),
                    );
                    return true;
                }
                stream.body_bytes_seen += n;
                let chunk = Bytes::copy_from_slice(&scratch[..n]);
                match tx.try_send(Ok(chunk)) {
                    Ok(()) => {}
                    Err(TrySendError::Full(item)) => {
                        if let Ok(b) = item {
                            stream.stalled = Some(b);
                        }
                        break;
                    }
                    Err(TrySendError::Closed(_)) => {
                        shutdown(conn, stream_id, quiche::Shutdown::Read, 0);
                        reset_upload_half(conn, stream_id, stream);
                        return true;
                    }
                }
            }
            Err(quiche::h3::Error::Done) => {
                drained_clean = true;
                break;
            }
            Err(e) => {
                deliver_stream_error(&tx, std::io::Error::other(format!("h3 recv_body: {e}")));
                return true;
            }
        }
    }

    if drained_clean
        && stream.stalled.is_none()
        && (stream.peer_finished || conn.stream_finished(stream_id))
    {
        if stream.length_mismatch() {
            deliver_stream_error(&tx, std::io::Error::other(LENGTH_MISMATCH));
        }
        stream.stream_tx = None;
        return true;
    }
    false
}

pub(in crate::quic::pool) fn pump_streaming_bodies(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
    scratch: &mut [u8],
    max_response_body_bytes: u64,
) -> bool {
    let mut to_remove: Vec<u64> = Vec::new();
    for (stream_id, stream) in streams.iter_mut() {
        if stream.stream_tx.is_none() {
            continue;
        }
        if forward_stream_body(
            h3,
            conn,
            *stream_id,
            stream,
            scratch,
            max_response_body_bytes,
        ) {
            to_remove.push(*stream_id);
        }
    }

    for id in to_remove {
        streams.remove(&id);
    }

    streams.values().any(|s| s.stalled.is_some())
}
