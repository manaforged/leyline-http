use tokio::sync::mpsc::error::TrySendError;

use super::*;

impl Drop for H3Stream {
    fn drop(&mut self) {
        if let Some(pump) = self.pump.take() {
            pump.abort();
        }
    }
}

impl H3Stream {
    pub(super) fn new(
        resp_tx: oneshot::Sender<Result<H3Response, H3SendError>>,
        body: Option<Bytes>,
        stream_tx: Option<mpsc::Sender<std::io::Result<Bytes>>>,
        streaming: bool,
    ) -> Self {
        let mut out_chunks = VecDeque::new();
        let (body_eof, fin_sent) = if streaming {
            (false, false)
        } else {
            match body {
                Some(b) if !b.is_empty() => {
                    out_chunks.push_back(b);
                    (true, false)
                }
                _ => (true, true),
            }
        };
        let terminal = stream_tx.as_ref().and_then(reserve_terminal);
        Self {
            resp_tx: Some(resp_tx),
            response: H3ResponseState::Initial,
            status: 0,
            headers: Vec::new(),
            trailers: Vec::new(),
            body: Vec::new(),
            mode: ResponseMode::from(stream_tx.is_some()),
            stream_tx,
            terminal,
            head_sent: false,
            stalled: None,
            peer_finished: false,
            body_bytes_seen: 0,
            declared_len: None,
            expects_body: true,
            out_chunks,
            out_offset: 0,
            body_eof,
            fin_sent,
            upload_credit: None,
            pump: None,
            retry: None,
        }
    }

    pub(super) fn cancel_upload(&mut self) {
        if let Some(pump) = self.pump.take() {
            pump.abort();
        }
        self.out_chunks.clear();
        self.out_offset = 0;
        self.fin_sent = true;
    }

    pub(super) fn is_streaming(&self) -> bool {
        self.stream_tx.is_some()
    }

    pub(super) fn body_write_pending(&self) -> bool {
        !self.out_chunks.is_empty() || (self.body_eof && !self.fin_sent)
    }

    pub(super) fn send_side_open(&self) -> bool {
        !self.fin_sent
    }

    pub(super) fn headers(&mut self, list: &[(String, String)]) -> Result<(), &'static str> {
        match self.response.headers(list)? {
            H3HeaderBlock::Informational => {}
            H3HeaderBlock::Final { status, headers } => {
                self.status = status;
                if !self.mode.keeps_stream(status) {
                    self.stream_tx = None;
                    self.terminal = None;
                }
                self.declared_len = headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.trim().parse().ok());
                self.headers = headers;
            }
            H3HeaderBlock::Trailers(headers) => self.trailers = headers,
        }
        Ok(())
    }

    pub(super) fn data(&self) -> Result<(), &'static str> {
        self.response.data()
    }

    pub(super) fn finish(&self) -> Result<(), &'static str> {
        self.response.finish()?;
        if !self.is_streaming() && self.length_mismatch() {
            return Err(LENGTH_MISMATCH);
        }
        Ok(())
    }

    pub(super) fn length_mismatch(&self) -> bool {
        self.expects_body
            && !matches!(self.status, 204 | 304)
            && self
                .declared_len
                .is_some_and(|len| len != self.body_bytes_seen as u64)
    }

    pub(super) fn deliver(&mut self, result: Result<H3Response, String>) {
        if let Some(tx) = self.resp_tx.take() {
            drop(tx.send(result.map_err(H3SendError::Failed)));
        }
    }

    pub(super) fn deliver_unsent(&mut self, message: String) {
        if let Some(tx) = self.resp_tx.take() {
            drop(tx.send(Err(H3SendError::NotSent(message))));
        }
    }

    pub(super) fn deliver_rejected(&mut self, message: String) {
        if let Some(tx) = self.resp_tx.take() {
            drop(tx.send(Err(H3SendError::Rejected(message))));
        }
    }

    pub(super) fn deliver_body_limit(&mut self, limit: BodyLimit) {
        if let Some(tx) = self.resp_tx.take() {
            drop(tx.send(Err(H3SendError::BodyLimit(limit))));
        }
    }

    pub(super) fn deliver_head(&mut self) {
        if let Some(tx) = self.resp_tx.take() {
            drop(tx.send(Ok(H3Response {
                status: self.status,
                headers: std::mem::take(&mut self.headers),
                body: Vec::new(),
                trailers: Vec::new(),
            })));
        }
        self.head_sent = true;
    }

    pub(super) fn deliver_request_body_error(&mut self, error: std::io::Error) {
        if self.head_sent {
            self.deliver_terminal(error);
        } else if let Some(tx) = self.resp_tx.take() {
            drop(tx.send(Err(H3SendError::RequestBody(error))));
        }
    }

    pub(super) fn deliver_error(&mut self, message: String) {
        if self.head_sent {
            self.deliver_terminal(std::io::Error::other(message));
        } else {
            self.deliver(Err(message));
        }
    }

    pub(super) fn deliver_terminal(&mut self, error: std::io::Error) {
        if let Some(permit) = self.terminal.take() {
            drop(permit.send(Err(error)));
        } else if let Some(tx) = &self.stream_tx {
            match tx.try_send(Err(error)) {
                Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Closed(_)) => {}
            }
        }
    }
}

fn reserve_terminal(
    tx: &mpsc::Sender<std::io::Result<Bytes>>,
) -> Option<mpsc::OwnedPermit<std::io::Result<Bytes>>> {
    match tx.clone().try_reserve_owned() {
        Ok(permit) => Some(permit),
        Err(TrySendError::Full(_)) | Err(TrySendError::Closed(_)) => None,
    }
}
