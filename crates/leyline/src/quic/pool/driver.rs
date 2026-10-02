use super::*;

mod drain;
mod event_loop;
mod forward;
mod upload;
use event_loop::H3Loop;
pub(super) use forward::*;
pub(super) use upload::*;

const GOAWAY_NOT_SENT: &str = "server sent GOAWAY: request not sent";

struct Drain<'a> {
    h3: &'a mut quiche::h3::Connection,
    conn: &'a mut quiche::Connection,
    streams: &'a mut HashMap<u64, H3Stream>,
    pending: &'a mut VecDeque<H3Command>,
    scratch: &'a mut [u8],
    max_body: u64,
    admit: &'a mut Option<usize>,
    draining: bool,
}

impl H3Driver {
    pub(super) async fn run(self) {
        let mut event_loop = H3Loop::new(self);
        while event_loop.turn().await.is_continue() {}
    }
}

pub(super) fn start_pending(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
    pending: &mut VecDeque<H3Command>,
    body_chunk_tx: &mpsc::Sender<H3BodyChunk>,
    admit_cap: Option<usize>,
    priority_update: bool,
) {
    while let Some(cmd) = pending.front() {
        if command_is_cancelled(cmd) {
            drop(pending.pop_front());
            continue;
        }
        let H3Command::Request {
            headers,
            body,
            body_stream,
            retried,
            ..
        } = cmd;
        if admit_cap.is_some_and(|cap| streams.len() >= cap) {
            break;
        }
        let streaming = body_stream.is_some();
        let fin = !streaming && body.as_ref().is_none_or(Bytes::is_empty);
        let retry = (!streaming && !*retried).then(|| (headers.clone(), body.clone()));
        let expects_body = !headers
            .iter()
            .any(|h| h.name() == b":method" && h.value() == b"HEAD");
        let priority = priority_update
            .then(|| headers.iter().find(|h| h.name() == b"priority"))
            .flatten()
            .map(|h| h.value().to_vec());
        match h3.send_request(conn, headers, fin) {
            Ok(stream_id) => {
                send_priority_update(h3, conn, stream_id, priority);
                let Some(H3Command::Request {
                    body,
                    body_stream,
                    stream_body_tx,
                    resp_tx,
                    ..
                }) = pending.pop_front()
                else {
                    unreachable!("front matched Request above");
                };
                let mut stream = H3Stream::new(resp_tx, body, stream_body_tx, streaming);
                stream.retry = retry;
                stream.expects_body = expects_body;
                if let Some(body_stream) = body_stream {
                    let credit = crate::util::upload::upload_credit();
                    let pump = tokio::spawn(crate::util::upload::pump_request_body(
                        stream_id,
                        body_stream,
                        body_chunk_tx.clone(),
                        Arc::clone(&credit),
                    ));
                    stream.upload_credit = Some(credit);
                    stream.pump = Some(pump.abort_handle());
                }
                if !write_request_body(h3, conn, stream_id, &mut stream) {
                    streams.insert(stream_id, stream);
                }
            }
            Err(quiche::h3::Error::StreamBlocked)
            | Err(quiche::h3::Error::Done)
            | Err(quiche::h3::Error::TransportError(quiche::Error::StreamLimit)) => break,
            Err(e) => {
                if let Some(H3Command::Request { resp_tx, .. }) = pending.pop_front() {
                    drop(resp_tx.send(Err(H3SendError::Failed(format!("h3 send_request: {e}")))));
                }
            }
        }
    }
}

fn send_priority_update(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    stream_id: u64,
    priority: Option<Vec<u8>>,
) {
    if let Some(value) = priority
        && let Err(e) = h3.send_priority_update_field_value(conn, stream_id, &value)
    {
        tracing::debug!(
            target: "leyline::quic",
            stream_id,
            error = %e,
            "h3 PRIORITY_UPDATE not sent"
        );
    }
}

fn shutdown(
    conn: &mut quiche::Connection,
    id: u64,
    dir: quiche::Shutdown,
    code: quiche::h3::WireErrorCode,
) {
    match conn.stream_shutdown(id, dir, code as u64) {
        Ok(()) | Err(quiche::Error::Done) => {}
        Err(e) => tracing::warn!(
            target: "leyline::quic",
            stream_id = id,
            error = %e,
            "h3 stream shutdown failed"
        ),
    }
}

fn reset_upload_half(
    conn: &mut quiche::Connection,
    stream_id: u64,
    stream: &mut H3Stream,
    code: quiche::h3::WireErrorCode,
) {
    if stream.send_side_open() {
        shutdown(conn, stream_id, quiche::Shutdown::Write, code);
    }
    stream.cancel_upload();
}

fn abort_stream(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    stream_id: u64,
    stream: &mut H3Stream,
    code: quiche::h3::WireErrorCode,
) {
    h3.cancel_stream(stream_id);
    shutdown(conn, stream_id, quiche::Shutdown::Read, code);
    reset_upload_half(conn, stream_id, stream, code);
}

fn stream_reset_message(code: u64) -> String {
    format!("h3 stream reset: {code}")
}

fn on_body_read_error(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    stream_id: u64,
    stream: &mut H3Stream,
    error: quiche::h3::Error,
) -> String {
    match error {
        quiche::h3::Error::TransportError(quiche::Error::StreamReset(code)) => {
            reset_upload_half(
                conn,
                stream_id,
                stream,
                quiche::h3::WireErrorCode::RequestCancelled,
            );
            stream_reset_message(code)
        }
        _ => {
            abort_stream(
                h3,
                conn,
                stream_id,
                stream,
                quiche::h3::WireErrorCode::GeneralProtocolError,
            );
            format!("h3 recv_body: {error}")
        }
    }
}

fn caller_gone(
    resp_tx: Option<&oneshot::Sender<Result<H3Response, H3SendError>>>,
    stream_tx: Option<&mpsc::Sender<std::io::Result<Bytes>>>,
) -> bool {
    match resp_tx {
        Some(tx) => tx.is_closed(),
        None => stream_tx.is_some_and(mpsc::Sender::is_closed),
    }
}

pub(super) fn stream_is_cancelled(stream: &H3Stream) -> bool {
    caller_gone(stream.resp_tx.as_ref(), stream.stream_tx.as_ref())
}

pub(super) fn command_is_cancelled(cmd: &H3Command) -> bool {
    let H3Command::Request { resp_tx, .. } = cmd;
    caller_gone(Some(resp_tx), None)
}

pub(super) fn cancelled_stream_ids(streams: &HashMap<u64, H3Stream>) -> Vec<u64> {
    streams
        .iter()
        .filter(|(_, s)| stream_is_cancelled(s))
        .map(|(&id, _)| id)
        .collect()
}

pub(super) fn sweep_cancelled_streams(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
    pending: &mut VecDeque<H3Command>,
) {
    pending.retain(|cmd| !command_is_cancelled(cmd));
    for id in cancelled_stream_ids(streams) {
        if let Some(mut stream) = streams.remove(&id) {
            abort_stream(
                h3,
                conn,
                id,
                &mut stream,
                quiche::h3::WireErrorCode::RequestCancelled,
            );
        }
    }
}

pub(super) fn deliver_stream_error(tx: &mpsc::Sender<std::io::Result<Bytes>>, err: std::io::Error) {
    let tx = tx.clone();
    tokio::spawn(async move {
        drop(tx.send(Err(err)).await);
    });
}

pub(super) fn fail_all(
    streams: &mut HashMap<u64, H3Stream>,
    pending: &mut VecDeque<H3Command>,
    commands: &mut mpsc::Receiver<H3Command>,
    closed: &AtomicBool,
    reason: String,
) {
    closed.store(true, Ordering::Release);
    commands.close();
    while let Ok(cmd) = commands.try_recv() {
        pending.push_back(cmd);
    }
    for (_, mut stream) in streams.drain() {
        stream.deliver_error(reason.clone());
    }
    for cmd in pending.drain(..) {
        reject_unsent(cmd, reason.clone());
    }
}

fn reject_unsent(cmd: H3Command, reason: String) {
    let H3Command::Request { resp_tx, .. } = cmd;
    drop(resp_tx.send(Err(H3SendError::NotSent(reason))));
}
