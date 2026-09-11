use super::*;

impl Drain<'_> {
    pub(super) fn run(&mut self) -> Result<bool, String> {
        loop {
            match self.h3.poll(self.conn) {
                Ok((id, quiche::h3::Event::Headers { list, .. })) => self.headers(id, &list),
                Ok((id, quiche::h3::Event::Data)) => self.data(id),
                Ok((id, quiche::h3::Event::Finished)) => self.finish(id),
                Ok((id, quiche::h3::Event::Reset(e))) => self.reset(id, e),
                Ok((_, quiche::h3::Event::PriorityUpdate)) => {}
                Ok((_, quiche::h3::Event::GoAway)) => return Ok(true),
                Err(quiche::h3::Error::Done) => return Ok(false),
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
            close(
                self.conn,
                id,
                quiche::h3::WireErrorCode::MessageError as u64,
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
            close(
                self.conn,
                id,
                quiche::h3::WireErrorCode::MessageError as u64,
            );
            stream.deliver_error(message.into());
            self.streams.remove(&id);
            return;
        }
        if stream.is_streaming() {
            if forward_stream_body(self.h3, self.conn, id, stream, self.scratch, self.max_body) {
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
        while let Ok(n) = self.h3.recv_body(self.conn, id, self.scratch) {
            if n == 0 {
                break;
            }
            if let Err(new_len) = check_body_budget(stream.body_bytes_seen, n, max) {
                shutdown(
                    self.conn,
                    id,
                    quiche::Shutdown::Read,
                    quiche::h3::WireErrorCode::ExcessiveLoad as u64,
                );
                stream.deliver(Err(format!(
                    "h3: response body exceeded max_response_body_bytes ({new_len} > {max})"
                )));
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
            close(
                self.conn,
                id,
                quiche::h3::WireErrorCode::MessageError as u64,
            );
            if let Some(mut stream) = self.streams.remove(&id) {
                stream.deliver_error(message.into());
            }
            return;
        }
        let streaming = self.streams.get(&id).map(H3Stream::is_streaming);
        match streaming {
            Some(true) => {
                if let Some(stream) = self.streams.get_mut(&id) {
                    reset_upload_half(self.conn, id, stream);
                    stream.peer_finished = true;
                }
            }
            Some(false) => {
                if let Some(mut stream) = self.streams.remove(&id) {
                    if stream.send_side_open() {
                        shutdown(self.conn, id, quiche::Shutdown::Write, 0);
                    }
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
        if e == 0x10b {
            let cur = self.streams.len().max(1);
            let next = match *self.admit {
                Some(c) => c.min(cur / 2).max(1),
                None => (cur / 2).max(1),
            };
            *self.admit = Some(next);
        }
        if let Some(mut stream) = self.streams.remove(&id)
            && e == 0x10b
            && !stream.head_sent
            && let Some((headers, body)) = stream.retry.take()
        {
            self.pending.push_back(H3Command::Request {
                headers,
                body,
                body_stream: None,
                stream_body_tx: None,
                resp_tx: stream.resp_tx.take().expect("buffered keeps resp_tx"),
                retried: true,
            });
            return;
        }
        if let Some(mut stream) = self.streams.remove(&id) {
            let msg = format!("h3 stream reset: {e}");
            if stream.head_sent {
                if let Some(tx) = &stream.stream_tx {
                    deliver_stream_error(tx, std::io::Error::other(msg));
                }
            } else {
                stream.deliver(Err(msg));
            }
        }
    }
}

fn close(conn: &mut quiche::Connection, id: u64, code: u64) {
    shutdown(conn, id, quiche::Shutdown::Read, code);
    shutdown(conn, id, quiche::Shutdown::Write, 0);
}
