use super::*;

fn headers(fields: &[(&str, &str)]) -> Vec<(String, String)> {
    fields
        .iter()
        .map(|(name, value)| ((*name).into(), (*value).into()))
        .collect()
}

#[tokio::test]
async fn streaming_head_waits_for_final_response_after_103() {
    let (tx, rx) = oneshot::channel();
    let (body_tx, _body_rx) = mpsc::channel(1);
    let mut stream = H3Stream::new(tx, None, Some(body_tx), false);

    stream
        .headers(&headers(&[(":status", "103"), ("link", "</style.css>")]))
        .expect("valid informational response");
    assert_eq!(stream.response, H3ResponseState::Initial);
    if stream.is_streaming() && stream.response == H3ResponseState::Final && !stream.head_sent {
        stream.deliver_head();
    }
    assert!(!stream.head_sent, "103 must not deliver a streaming head");

    stream
        .headers(&headers(&[
            (":status", "200"),
            ("content-type", "text/plain"),
        ]))
        .expect("valid final response");
    if stream.is_streaming() && stream.response == H3ResponseState::Final && !stream.head_sent {
        stream.deliver_head();
    }

    let head = rx
        .await
        .expect("final response head delivered")
        .expect("success");
    assert_eq!(head.status, 200);
    assert_eq!(head.headers, headers(&[("content-type", "text/plain")]));
}

#[test]
fn trailers_are_regular_fields_after_final_response() {
    let mut state = H3ResponseState::Initial;
    assert!(matches!(
        state.headers(&headers(&[(":status", "200")])),
        Ok(H3HeaderBlock::Final { .. })
    ));
    assert!(matches!(
        state.headers(&headers(&[("x-checksum", "abc")])),
        Ok(H3HeaderBlock::Trailers(fields)) if fields == headers(&[("x-checksum", "abc")])
    ));
    assert_eq!(state, H3ResponseState::Trailers);
}

#[test]
fn response_head_rejects_duplicate_and_malformed_status() {
    let mut state = H3ResponseState::Initial;
    assert!(
        state
            .headers(&headers(&[(":status", "200"), (":status", "201")]))
            .is_err()
    );

    let mut state = H3ResponseState::Initial;
    assert!(state.headers(&headers(&[(":status", "20")])).is_err());

    let mut state = H3ResponseState::Initial;
    assert!(state.headers(&headers(&[(":status", "101")])).is_err());
}

#[test]
fn trailers_reject_status_pseudo_header() {
    let mut state = H3ResponseState::Initial;
    state
        .headers(&headers(&[(":status", "200")]))
        .expect("final response");
    assert!(state.headers(&headers(&[(":status", "204")])).is_err());
}

#[test]
fn data_requires_final_response_head() {
    assert!(H3ResponseState::Initial.data().is_err());
}

#[test]
fn empty_and_absent_bodies_have_nothing_to_send() {
    let (tx, _rx) = oneshot::channel();
    let s = H3Stream::new(tx, None, None, false);
    assert!(!s.body_write_pending());
    assert!(!s.send_side_open(), "no body → FIN already rode HEADERS");

    let (tx, _rx) = oneshot::channel();
    let s = H3Stream::new(tx, Some(Bytes::new()), None, false);
    assert!(
        !s.body_write_pending(),
        "an empty Bytes body must not park as pending"
    );
    assert!(!s.send_side_open());

    let (tx, _rx) = oneshot::channel();
    let s = H3Stream::new(tx, Some(Bytes::from_static(b"x")), None, false);
    assert!(s.body_write_pending());
    assert!(s.send_side_open(), "buffered body's FIN not sent yet");
}

#[test]
fn streaming_body_pends_on_chunk_and_eof() {
    let (tx, _rx) = oneshot::channel();
    let mut s = H3Stream::new(tx, None, None, true);
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
    let mut s = H3Stream::new(tx, None, None, true);
    s.out_chunks
        .push_back(Bytes::from_static(b"queued upload bytes"));
    assert!(s.body_write_pending());
    assert!(s.send_side_open());

    s.cancel_upload();
    assert!(s.out_chunks.is_empty());
    assert!(!s.body_write_pending());
    assert!(
        !s.send_side_open(),
        "send side marked finished after cancel"
    );
}

#[tokio::test]
async fn deliver_is_once_only() {
    let (tx, rx) = oneshot::channel();
    let mut stream = H3Stream::new(tx, None, None, false);
    stream.status = 200;
    stream.body.extend_from_slice(b"hello");
    let resp = H3Response {
        status: stream.status,
        headers: std::mem::take(&mut stream.headers),
        body: std::mem::take(&mut stream.body),
        trailers: Vec::new(),
    };
    stream.deliver(Ok(resp));
    stream.deliver(Err("late teardown".into()));

    let got = rx.await.expect("sender delivered").expect("ok response");
    assert_eq!(got.status, 200);
    assert_eq!(got.body, b"hello");
}

#[test]
fn cancelled_stream_ids_selects_only_dropped_receivers() {
    let mut streams = HashMap::new();
    let (tx_live, _rx_live) = oneshot::channel::<Result<H3Response, H3SendError>>();
    streams.insert(1u64, H3Stream::new(tx_live, None, None, false));
    let (tx_dead, rx_dead) = oneshot::channel::<Result<H3Response, H3SendError>>();
    streams.insert(2u64, H3Stream::new(tx_dead, None, None, false));
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
    let (tx, rx) = oneshot::channel::<Result<H3Response, H3SendError>>();
    let (body_tx, body_rx) = mpsc::channel(4);
    let mut s = H3Stream::new(tx, None, Some(body_tx), true);
    assert!(
        !stream_is_cancelled(&s),
        "live resp receiver → not cancelled"
    );
    drop(rx);
    assert!(
        stream_is_cancelled(&s),
        "dropped resp receiver pre-head → cancelled"
    );

    let (tx2, _rx2) = oneshot::channel::<Result<H3Response, H3SendError>>();
    s.resp_tx = Some(tx2);
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
    assert!(H3SendError::NotSent("connection closed".into()).is_retryable());
    assert!(!H3SendError::Failed("stream reset".into()).is_retryable());
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
    streams.insert(1u64, H3Stream::new(tx, None, None, false));

    let mut pending = VecDeque::new();
    let (tx2, rx_pending) = oneshot::channel();
    pending.push_back(H3Command::Request {
        headers: Vec::new(),
        body: None,
        body_stream: None,
        stream_body_tx: None,
        resp_tx: tx2,
        retried: false,
    });

    fail_all(&mut streams, &mut pending, &closed, "boom".into());

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
