use super::*;

impl Drain<'_> {
    pub(super) fn run(&mut self) -> Result<bool, String> {
        let mut goaway = false;
        loop {
            match self.h3.poll(self.conn) {
                Ok((id, quiche::h3::Event::Headers { list, .. })) => self.headers(id, &list),
                Ok((id, quiche::h3::Event::Data)) => self.data(id),
                Ok((id, quiche::h3::Event::Finished)) => self.finish(id),
                Ok((id, quiche::h3::Event::Reset(e))) => self.reset(id, e),
                Ok((_, quiche::h3::Event::PriorityUpdate)) => {}
                Ok((last_id, quiche::h3::Event::GoAway)) => {
                    goaway = true;
                    self.goaway(last_id);
                }
                Err(quiche::h3::Error::Done) => return Ok(goaway),
                Err(e) => return Err(format!("h3 poll: {e}")),
            }
        }
    }

    fn headers(&mut self, id: u64, list: &[quiche::h3::Header]) {
        let list = list
            .iter()
            .map(|header| {
                (
                    String::from_utf8_lossy(header.name()).to_string(),
                    String::from_utf8_lossy(header.value()).to_string(),
                )
            })
            .collect::<Vec<_>>();
        let Some(stream) = self.streams.get_mut(&id) else {
            return;
        };
        if let Err(message) = stream.headers(&list) {
            abort_stream(
                self.h3,
                self.conn,
                id,
                stream,
                quiche::h3::WireErrorCode::GeneralProtocolError,
            );
            stream.deliver_error(message.into());
            self.streams.remove(&id);
            return;
        }
        if stream.is_streaming() && stream.response == H3ResponseState::Final && !stream.head_sent {
            stream.deliver_head();
        }
    }

    fn data(&mut self, id: u64) {
        let Some(stream) = self.streams.get_mut(&id) else {
            self.discard(id);
            return;
        };
        if let Err(message) = stream.data() {
            abort_stream(
                self.h3,
                self.conn,
                id,
                stream,
                quiche::h3::WireErrorCode::GeneralProtocolError,
            );
            stream.deliver_error(message.into());
            self.streams.remove(&id);
            return;
        }
        if stream.is_streaming() {
            if forward_stream_body(self.h3, self.conn, id, stream, self.scratch) {
                self.streams.remove(&id);
            }
            return;
        }
        self.buffer(id);
    }

    fn discard(&mut self, id: u64) {
        while let Ok(n) = self.h3.recv_body(self.conn, id, self.scratch) {
            if n == 0 {
                break;
            }
        }
    }

    fn buffer(&mut self, id: u64) {
        let max = self.max_body;
        let Some(stream) = self.streams.get_mut(&id) else {
            return;
        };
        loop {
            let n = match self.h3.recv_body(self.conn, id, self.scratch) {
                Ok(0) | Err(quiche::h3::Error::Done) => break,
                Ok(n) => n,
                Err(e) => {
                    let message = on_body_read_error(self.h3, self.conn, id, stream, e);
                    stream.deliver_error(message);
                    self.streams.remove(&id);
                    break;
                }
            };
            if check_body_budget(stream.body_bytes_seen, n, max).is_err() {
                abort_stream(
                    self.h3,
                    self.conn,
                    id,
                    stream,
                    quiche::h3::WireErrorCode::RequestCancelled,
                );
                stream.deliver_body_limit(BodyLimit(usize::try_from(max).unwrap_or(usize::MAX)));
                self.streams.remove(&id);
                break;
            }
            stream.body_bytes_seen += n;
            stream.body.extend_from_slice(&self.scratch[..n]);
        }
    }

    fn finish(&mut self, id: u64) {
        let invalid = self
            .streams
            .get(&id)
            .and_then(|stream| stream.finish().err());
        if let Some(message) = invalid {
            if let Some(mut stream) = self.streams.remove(&id) {
                abort_stream(
                    self.h3,
                    self.conn,
                    id,
                    &mut stream,
                    quiche::h3::WireErrorCode::GeneralProtocolError,
                );
                stream.deliver_error(message.into());
            }
            return;
        }
        let streaming = self.streams.get(&id).map(H3Stream::is_streaming);
        match streaming {
            Some(true) => {
                if let Some(stream) = self.streams.get_mut(&id) {
                    reset_upload_half(
                        self.conn,
                        id,
                        stream,
                        quiche::h3::WireErrorCode::RequestCancelled,
                    );
                    stream.peer_finished = true;
                }
            }
            Some(false) => {
                if let Some(mut stream) = self.streams.remove(&id) {
                    reset_upload_half(
                        self.conn,
                        id,
                        &mut stream,
                        quiche::h3::WireErrorCode::RequestCancelled,
                    );
                    let resp = H3Response {
                        status: stream.status,
                        headers: std::mem::take(&mut stream.headers),
                        body: std::mem::take(&mut stream.body),
                        trailers: std::mem::take(&mut stream.trailers),
                    };
                    stream.deliver(Ok(resp));
                }
            }
            None => {}
        }
    }

    fn reset(&mut self, id: u64, e: u64) {
        let rejected = e == quiche::h3::WireErrorCode::RequestRejected as u64;
        if rejected {
            let cur = self.streams.len().max(1);
            let next = match *self.admit {
                Some(c) => c.min(cur / 2).max(1),
                None => (cur / 2).max(1),
            };
            *self.admit = Some(next);
        }
        let Some(mut stream) = self.streams.remove(&id) else {
            return;
        };
        reset_upload_half(
            self.conn,
            id,
            &mut stream,
            quiche::h3::WireErrorCode::RequestCancelled,
        );
        if rejected
            && stream.response == H3ResponseState::Initial
            && let Some((headers, body)) = stream.retry.take()
            && let Some(resp_tx) = stream.resp_tx.take()
        {
            let cmd = H3Command::Request {
                headers,
                body,
                body_stream: None,
                stream_body_tx: stream.stream_tx.take(),
                resp_tx,
                retried: true,
            };
            if self.draining {
                reject_unsent(cmd, GOAWAY_NOT_SENT.into());
            } else {
                self.pending.push_back(cmd);
            }
            return;
        }
        stream.deliver_error(stream_reset_message(e));
    }

    fn goaway(&mut self, last_id: u64) {
        let rejected: Vec<u64> = self
            .streams
            .keys()
            .copied()
            .filter(|&id| id >= last_id)
            .collect();
        for id in rejected {
            if let Some(mut stream) = self.streams.remove(&id) {
                abort_stream(
                    self.h3,
                    self.conn,
                    id,
                    &mut stream,
                    quiche::h3::WireErrorCode::RequestCancelled,
                );
                if stream.head_sent {
                    stream.deliver_error("server sent GOAWAY: stream rejected".into());
                } else {
                    stream.deliver_unsent("server sent GOAWAY: request not processed".into());
                }
            }
        }
    }
}
