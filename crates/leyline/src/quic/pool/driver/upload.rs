use super::*;

pub(in crate::quic::pool) fn write_pending_request_bodies(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
) {
    for (stream_id, stream) in streams.iter_mut() {
        if stream.body_write_pending() {
            write_request_body(h3, conn, *stream_id, stream);
        }
    }
}

pub(in crate::quic::pool) fn write_request_body(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    stream_id: u64,
    stream: &mut H3Stream,
) {
    while let Some(front) = stream.out_chunks.front() {
        let remaining = &front[stream.out_offset..];
        let last_chunk = stream.body_eof && stream.out_chunks.len() == 1;
        match h3.send_body(conn, stream_id, remaining, last_chunk) {
            Ok(0) => return,
            Ok(written) => {
                stream.out_offset += written;
                if let Some(credit) = &stream.upload_credit {
                    credit.add_permits(written);
                }
                if stream.out_offset >= front.len() {
                    stream.out_chunks.pop_front();
                    stream.out_offset = 0;
                    if last_chunk {
                        stream.fin_sent = true;
                    }
                }
            }
            Err(quiche::h3::Error::Done) | Err(quiche::h3::Error::StreamBlocked) => return,
            Err(e) => {
                stream.deliver(Err(format!("h3 send_body: {e}")));
                stream.retry = None;
                stream.out_chunks.clear();
                stream.out_offset = 0;
                stream.fin_sent = true;
                return;
            }
        }
    }

    if stream.body_eof && !stream.fin_sent {
        match h3.send_body(conn, stream_id, &[], true) {
            Ok(_) => stream.fin_sent = true,
            Err(quiche::h3::Error::Done) | Err(quiche::h3::Error::StreamBlocked) => {}
            Err(e) => {
                stream.deliver(Err(format!("h3 send_body fin: {e}")));
                stream.retry = None;
                stream.fin_sent = true;
            }
        }
    }
}

pub(super) async fn pump_request_body(
    stream_id: u64,
    mut body: H3RequestBodyStream,
    tx: mpsc::Sender<H3BodyChunk>,
    credit: Arc<Semaphore>,
) {
    use futures_util::StreamExt;
    while let Some(item) = body.next().await {
        match item {
            Ok(mut data) => {
                while !data.is_empty() {
                    let take = data.len().min(UPLOAD_CHUNK);
                    let slice = data.split_to(take);
                    let Ok(permit) = credit.acquire_many(take as u32).await else {
                        return;
                    };
                    permit.forget();
                    if tx
                        .send(H3BodyChunk::Chunk {
                            stream_id,
                            data: slice,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
            Err(error) => {
                drop(
                    tx.send(H3BodyChunk::Eof {
                        stream_id,
                        error: Some(error),
                    })
                    .await,
                );
                return;
            }
        }
    }
    drop(
        tx.send(H3BodyChunk::Eof {
            stream_id,
            error: None,
        })
        .await,
    );
}

pub(in crate::quic::pool) fn on_request_body_chunk(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
    chunk: H3BodyChunk,
) {
    match chunk {
        H3BodyChunk::Chunk { stream_id, data } => {
            if let Some(stream) = streams.get_mut(&stream_id) {
                stream.out_chunks.push_back(data);
            }
        }
        H3BodyChunk::Eof { stream_id, error } => {
            let Some(stream) = streams.get_mut(&stream_id) else {
                return;
            };
            match error {
                None => stream.body_eof = true,
                Some(e) => {
                    shutdown(conn, stream_id, quiche::Shutdown::Write, 0);
                    shutdown(conn, stream_id, quiche::Shutdown::Read, 0);
                    h3.cancel_stream(stream_id);
                    let msg = format!("h3 request body stream error: {e}");
                    if stream.head_sent {
                        if let Some(tx) = &stream.stream_tx {
                            deliver_stream_error(tx, std::io::Error::other(msg));
                        }
                    } else {
                        stream.deliver(Err(msg));
                    }
                    streams.remove(&stream_id);
                }
            }
        }
    }
}
