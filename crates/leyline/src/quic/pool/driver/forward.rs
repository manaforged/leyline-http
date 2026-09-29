use tokio::sync::mpsc::error::TrySendError;

use super::*;

enum Push {
    Sent,
    Stalled,
    Closed,
}

enum Pull {
    Drained,
    Stalled,
    Closed,
}

pub(in crate::quic::pool) fn forward_stream_body(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    stream_id: u64,
    stream: &mut H3Stream,
    scratch: &mut [u8],
) -> bool {
    let Some(tx) = stream.stream_tx.clone() else {
        return false;
    };

    if let Some(chunk) = stream.stalled.take() {
        match push_chunk(h3, conn, stream_id, stream, &tx, chunk) {
            Push::Sent => {}
            Push::Stalled => return false,
            Push::Closed => return true,
        }
    }

    match pull_body(h3, conn, stream_id, stream, scratch, &tx) {
        Pull::Drained => finish_stream(conn, stream_id, stream, &tx),
        Pull::Stalled => false,
        Pull::Closed => true,
    }
}

fn push_chunk(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    stream_id: u64,
    stream: &mut H3Stream,
    tx: &mpsc::Sender<std::io::Result<Bytes>>,
    chunk: Bytes,
) -> Push {
    match tx.try_send(Ok(chunk)) {
        Ok(()) => Push::Sent,
        Err(TrySendError::Full(item)) => {
            if let Ok(returned) = item {
                stream.stalled = Some(returned);
            }
            Push::Stalled
        }
        Err(TrySendError::Closed(_)) => {
            abort_stream(
                h3,
                conn,
                stream_id,
                stream,
                quiche::h3::WireErrorCode::RequestCancelled,
            );
            Push::Closed
        }
    }
}

fn pull_body(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    stream_id: u64,
    stream: &mut H3Stream,
    scratch: &mut [u8],
    tx: &mpsc::Sender<std::io::Result<Bytes>>,
) -> Pull {
    loop {
        match h3.recv_body(conn, stream_id, scratch) {
            Ok(0) | Err(quiche::h3::Error::Done) => return Pull::Drained,
            Ok(n) => {
                stream.body_bytes_seen += n;
                let chunk = Bytes::copy_from_slice(&scratch[..n]);
                match push_chunk(h3, conn, stream_id, stream, tx, chunk) {
                    Push::Sent => {}
                    Push::Stalled => return Pull::Stalled,
                    Push::Closed => return Pull::Closed,
                }
            }
            Err(e) => {
                let message = on_body_read_error(h3, conn, stream_id, stream, e);
                deliver_stream_error(tx, std::io::Error::other(message));
                return Pull::Closed;
            }
        }
    }
}

fn finish_stream(
    conn: &mut quiche::Connection,
    stream_id: u64,
    stream: &mut H3Stream,
    tx: &mpsc::Sender<std::io::Result<Bytes>>,
) -> bool {
    let finished = stream.stalled.is_none()
        && (stream.peer_finished
            || (conn.stream_finished(stream_id) && !conn.stream_readable(stream_id)));
    if !finished {
        return false;
    }
    reset_upload_half(
        conn,
        stream_id,
        stream,
        quiche::h3::WireErrorCode::RequestCancelled,
    );
    if stream.length_mismatch() {
        deliver_stream_error(tx, std::io::Error::other(LENGTH_MISMATCH));
    }
    stream.stream_tx = None;
    true
}

pub(in crate::quic::pool) fn pump_streaming_bodies(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
    scratch: &mut [u8],
) -> bool {
    let mut to_remove: Vec<u64> = Vec::new();
    for (stream_id, stream) in streams.iter_mut() {
        if stream.stream_tx.is_none() {
            continue;
        }
        if forward_stream_body(h3, conn, *stream_id, stream, scratch) {
            to_remove.push(*stream_id);
        }
    }

    for id in to_remove {
        streams.remove(&id);
    }

    streams.values().any(|s| s.stalled.is_some())
}
