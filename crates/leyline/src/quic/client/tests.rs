use super::driver::{cancelled_stream_ids, command_is_cancelled, fail_all, stream_is_cancelled};
use super::*;

fn headers(fields: &[(&str, &str)]) -> Vec<quiche::h3::Header> {
    fields
        .iter()
        .map(|(name, value)| quiche::h3::Header::new(name.as_bytes(), value.as_bytes()))
        .collect()
}

fn typed(fields: &[(&'static str, &'static str)]) -> http::HeaderMap {
    fields
        .iter()
        .map(|(name, value)| {
            (
                http::HeaderName::from_static(name),
                http::HeaderValue::from_static(value),
            )
        })
        .collect()
}

#[test]
fn trailers_are_regular_fields_after_final_response() {
    let mut state = ResponseState::Initial(0);
    assert!(matches!(
        response_block(&mut state, &headers(&[(":status", "200")])),
        Ok(ResponseBlock::Final { .. })
    ));
    assert!(matches!(
        response_block(&mut state, &headers(&[("x-checksum", "abc")])),
        Ok(ResponseBlock::Trailers(fields)) if fields == typed(&[("x-checksum", "abc")])
    ));
    assert_eq!(state, ResponseState::Trailers);
}

#[test]
fn response_head_rejects_duplicate_and_malformed_status() {
    let mut state = ResponseState::Initial(0);
    assert!(
        response_block(
            &mut state,
            &headers(&[(":status", "200"), (":status", "201")])
        )
        .is_err()
    );

    let mut state = ResponseState::Initial(0);
    assert!(response_block(&mut state, &headers(&[(":status", "20")])).is_err());

    let mut state = ResponseState::Initial(0);
    assert!(response_block(&mut state, &headers(&[(":status", "101")])).is_err());
}

#[test]
fn trailers_reject_status_pseudo_header() {
    let mut state = ResponseState::Initial(0);
    response_block(&mut state, &headers(&[(":status", "200")])).expect("final response");
    assert!(response_block(&mut state, &headers(&[(":status", "204")])).is_err());
}

#[test]
fn data_requires_final_response_head() {
    assert!(ResponseState::Initial(0).data().is_err());
}

#[test]
fn empty_and_absent_bodies_have_nothing_to_send() {
    let (tx, _rx) = oneshot::channel();
    let s = H3Stream::new(
        tx,
        None,
        None,
        true,
        crate::wire::ResponseMode::Buffered,
        false,
        crate::error::BodyLimit::session(crate::wire::DEFAULT_MAX_BODY_SIZE),
    );
    assert!(!s.body_write_pending());
    assert!(!s.send_side_open(), "no body → FIN already rode HEADERS");

    let (tx, _rx) = oneshot::channel();
    let s = H3Stream::new(
        tx,
        Some(Bytes::new()),
        None,
        true,
        crate::wire::ResponseMode::Buffered,
        false,
        crate::error::BodyLimit::session(crate::wire::DEFAULT_MAX_BODY_SIZE),
    );
    assert!(
        !s.body_write_pending(),
        "an empty Bytes body must not park as pending"
    );
    assert!(!s.send_side_open());

    let (tx, _rx) = oneshot::channel();
    let s = H3Stream::new(
        tx,
        Some(Bytes::from_static(b"x")),
        None,
        false,
        crate::wire::ResponseMode::Buffered,
        false,
        crate::error::BodyLimit::session(crate::wire::DEFAULT_MAX_BODY_SIZE),
    );
    assert!(s.body_write_pending());
    assert!(s.send_side_open(), "buffered body's FIN not sent yet");
}

#[test]
fn streaming_body_pends_on_chunk_and_eof() {
    let (tx, _rx) = oneshot::channel();
    let mut s = H3Stream::new(
        tx,
        None,
        None,
        false,
        crate::wire::ResponseMode::Buffered,
        false,
        crate::error::BodyLimit::session(crate::wire::DEFAULT_MAX_BODY_SIZE),
    );
    assert!(!s.body_write_pending(), "no chunks yet → nothing to write");
    assert!(s.send_side_open(), "streaming send side open until EOF");

    s.out_chunks.push_back(Bytes::from_static(b"chunk"));
    assert!(s.body_write_pending(), "queued chunk must pend");

    s.out_chunks.clear();
    s.body_eof = true;
    assert!(
        s.body_write_pending(),
        "EOF with an empty queue still pends an empty FIN"
    );
    assert!(s.send_side_open(), "FIN not actually sent until written");

    s.fin_sent = true;
    assert!(!s.body_write_pending());
    assert!(!s.send_side_open());
}

#[test]
fn cancel_upload_drops_queue_and_finishes_send_side() {
    let (tx, _rx) = oneshot::channel();
    let mut s = H3Stream::new(
        tx,
        Some(Bytes::from_static(b"queued upload bytes")),
        None,
        false,
        crate::wire::ResponseMode::Buffered,
        false,
        crate::error::BodyLimit::session(crate::wire::DEFAULT_MAX_BODY_SIZE),
    );
    assert!(s.body_write_pending());
    assert!(s.send_side_open());

    s.cancel_upload();
    assert!(!s.body_write_pending());
    assert!(
        !s.send_side_open(),
        "send side marked finished after cancel"
    );
}

#[tokio::test]
async fn deliver_is_once_only() {
    let (tx, rx) = oneshot::channel();
    let mut stream = H3Stream::new(
        tx,
        None,
        None,
        true,
        crate::wire::ResponseMode::Buffered,
        false,
        crate::error::BodyLimit::session(crate::wire::DEFAULT_MAX_BODY_SIZE),
    );
    let resp = stream.response(b"hello".to_vec());
    stream.reply(Ok(resp));
    stream.reply(Err(H3SendError::Failed("late teardown".into())));

    let got = rx.await.expect("sender delivered").expect("ok response");
    assert_eq!(got.status, 200);
    assert!(matches!(got.body, TransportBody::Buffered(body) if body == b"hello"));
}

#[test]
fn cancelled_stream_ids_selects_only_dropped_receivers() {
    let mut streams = HashMap::new();
    let (tx_live, _rx_live) = oneshot::channel::<Result<Box<TransportResponse>, H3SendError>>();
    streams.insert(
        1u64,
        H3Stream::new(
            tx_live,
            None,
            None,
            true,
            crate::wire::ResponseMode::Buffered,
            false,
            crate::error::BodyLimit::session(crate::wire::DEFAULT_MAX_BODY_SIZE),
        ),
    );
    let (tx_dead, rx_dead) = oneshot::channel::<Result<Box<TransportResponse>, H3SendError>>();
    streams.insert(
        2u64,
        H3Stream::new(
            tx_dead,
            None,
            None,
            true,
            crate::wire::ResponseMode::Buffered,
            false,
            crate::error::BodyLimit::session(crate::wire::DEFAULT_MAX_BODY_SIZE),
        ),
    );
    drop(rx_dead);

    let cancelled = cancelled_stream_ids(&streams);
    assert_eq!(
        cancelled,
        vec![2u64],
        "only the dropped-receiver stream is selected for reaping"
    );
}

#[tokio::test]
async fn cancellation_tracks_resp_then_body_receiver_across_the_head() {
    let (tx, rx) = oneshot::channel::<Result<Box<TransportResponse>, H3SendError>>();
    let (body_tx, terminal, body_rx) = crate::wire::body_channel();
    let mut s = H3Stream::new(
        tx,
        None,
        Some((body_tx, terminal)),
        false,
        crate::wire::ResponseMode::Streamed,
        false,
        crate::error::BodyLimit::session(crate::wire::DEFAULT_MAX_BODY_SIZE),
    );
    assert!(
        !stream_is_cancelled(&s),
        "live resp receiver → not cancelled"
    );
    drop(rx);
    assert!(
        stream_is_cancelled(&s),
        "dropped resp receiver pre-head → cancelled"
    );

    s.deliver_head();
    assert!(
        !stream_is_cancelled(&s),
        "live body receiver post-head → not cancelled"
    );
    drop(body_rx);
    assert!(
        stream_is_cancelled(&s),
        "dropped body receiver post-head → cancelled"
    );
}

#[test]
fn only_provably_unsent_requests_are_retryable() {
    assert_eq!(
        H3SendError::NotSent("connection closed".into()).failure(),
        Failure::Unsent
    );
    assert_eq!(
        H3SendError::Failed("stream reset".into()).failure(),
        Failure::Request
    );
    assert_eq!(
        H3SendError::Failed("stream reset".into()).message(),
        "stream reset"
    );
}

#[test]
fn fail_all_drains_streams_and_pending_and_marks_closed() {
    let closed = AtomicBool::new(false);
    let mut streams = HashMap::new();
    let (tx, rx_stream) = oneshot::channel();
    streams.insert(
        1u64,
        H3Stream::new(
            tx,
            None,
            None,
            true,
            crate::wire::ResponseMode::Buffered,
            false,
            crate::error::BodyLimit::session(crate::wire::DEFAULT_MAX_BODY_SIZE),
        ),
    );

    let mut pending = VecDeque::new();
    let (tx2, rx_pending) = oneshot::channel();
    pending.push_back(H3Command::Request {
        headers: Vec::new(),
        body: None,
        body_stream: None,
        stream_body: None,
        mode: crate::wire::ResponseMode::Buffered,
        head_request: false,
        body_limit: crate::error::BodyLimit::session(crate::wire::DEFAULT_MAX_BODY_SIZE),
        resp_tx: tx2,
    });

    let (_commands_tx, mut commands) = mpsc::channel(1);
    fail_all(
        &mut streams,
        &mut pending,
        &mut commands,
        &closed,
        "boom".into(),
    );

    assert!(closed.load(Ordering::Acquire));
    assert!(streams.is_empty());
    assert!(pending.is_empty());
    assert_eq!(
        rx_stream.blocking_recv().unwrap().unwrap_err().message(),
        "boom"
    );
    assert_eq!(
        rx_pending.blocking_recv().unwrap().unwrap_err().message(),
        "boom"
    );
}

#[test]
fn fail_all_answers_queued_commands_as_not_sent() {
    let closed = AtomicBool::new(false);
    let (commands_tx, mut commands) = mpsc::channel(1);
    let (resp_tx, queued) = oneshot::channel();
    commands_tx
        .try_send(H3Command::Request {
            headers: Vec::new(),
            body: None,
            body_stream: None,
            stream_body: None,
            mode: crate::wire::ResponseMode::Buffered,
            head_request: false,
            body_limit: crate::error::BodyLimit::session(crate::wire::DEFAULT_MAX_BODY_SIZE),
            resp_tx,
        })
        .unwrap();
    fail_all(
        &mut HashMap::new(),
        &mut VecDeque::new(),
        &mut commands,
        &closed,
        "boom".into(),
    );
    let err = queued.blocking_recv().unwrap().unwrap_err();
    assert_eq!(err.failure(), Failure::Unsent, "{}", err.message());
}

#[test]
fn a_queued_request_whose_caller_left_is_cancelled() {
    let (resp_tx, resp_rx) = oneshot::channel();
    let cmd = H3Command::Request {
        headers: Vec::new(),
        body: None,
        body_stream: None,
        stream_body: None,
        mode: crate::wire::ResponseMode::Buffered,
        head_request: false,
        body_limit: crate::error::BodyLimit::session(crate::wire::DEFAULT_MAX_BODY_SIZE),
        resp_tx,
    };
    assert!(!command_is_cancelled(&cmd));
    drop(resp_rx);
    assert!(command_is_cancelled(&cmd));
}

#[test]
fn endless_informational_responses_fail_the_stream() {
    let mut state = ResponseState::Initial(0);
    let early = headers(&[(":status", "103")]);
    for _ in 0..16 {
        assert!(matches!(
            response_block(&mut state, &early),
            Ok(ResponseBlock::Informational)
        ));
    }
    let Err(_) = response_block(&mut state, &early) else {
        panic!("the 17th informational response must fail the stream");
    };
}

#[test]
fn a_status_below_100_is_malformed() {
    let mut state = ResponseState::Initial(0);
    assert!(response_block(&mut state, &headers(&[(":status", "099")])).is_err());
}

#[test]
fn a_header_value_keeps_its_raw_bytes() {
    let mut state = ResponseState::Initial(0);
    let list = [
        quiche::h3::Header::new(b":status", b"200"),
        quiche::h3::Header::new(b"x-raw", &[b'a', 0xff]),
    ];
    let Ok(ResponseBlock::Final { headers, .. }) = response_block(&mut state, &list) else {
        panic!("final response head");
    };
    assert_eq!(headers["x-raw"].as_bytes(), [b'a', 0xff]);
}
