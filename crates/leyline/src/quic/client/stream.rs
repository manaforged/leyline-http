use tokio::sync::mpsc::error::TrySendError;

use super::*;

impl H3Stream {
    pub(super) fn new(
        resp_tx: oneshot::Sender<Result<TransportResponse, H3SendError>>,
        body: Option<Bytes>,
        stream_body: Option<StreamBody>,
        fin: bool,
        mode: ResponseMode,
        head_request: bool,
        body_limit: BodyLimit,
    ) -> Self {
        let mut out_chunks = VecDeque::new();
        let (body_eof, fin_sent) = match body {
            _ if fin => (true, true),
            Some(b) => {
                out_chunks.push_back(b);
                (true, false)
            }
            None => (false, false),
        };
        let (stream_tx, terminal) = stream_body.unzip();
        Self {
            resp_tx: Some(resp_tx),
            response: ResponseState::default(),
            status: http::StatusCode::OK,
            headers: http::HeaderMap::new(),
            trailers: http::HeaderMap::new(),
            body: Vec::new(),
            mode,
            stream_tx,
            terminal,
            head_sent: false,
            stalled: None,
            peer_finished: false,
            body_bytes_seen: 0,
            declared_len: None,
            head_request,
            body_limit,
            out_chunks,
            out_offset: 0,
            body_eof,
            fin_sent,
            upload: None,
        }
    }

    pub(super) fn cancel_upload(&mut self) {
        self.upload = None;
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

    pub(super) fn headers(&mut self, list: &[quiche::h3::Header]) -> Result<(), &'static str> {
        match response_block(&mut self.response, list)? {
            ResponseBlock::Informational => {}
            ResponseBlock::Final {
                status,
                headers,
                length,
            } => {
                self.status = status;
                if !self.mode.keeps_stream(status.as_u16()) {
                    self.stream_tx = None;
                    self.terminal = None;
                }
                self.declared_len = length;
                self.headers = headers;
            }
            ResponseBlock::Trailers(headers) => self.trailers = headers,
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
        crate::wire::length_mismatch(
            crate::wire::has_body(self.head_request, self.status.as_u16()),
            self.declared_len,
            self.body_bytes_seen as u64,
        )
    }

    pub(super) fn response(&mut self, body: Vec<u8>) -> TransportResponse {
        TransportResponse::new(
            http::Version::HTTP_3,
            self.status,
            std::mem::take(&mut self.headers).into(),
            std::mem::take(&mut self.trailers),
            TransportBody::Buffered(body),
        )
    }

    pub(super) fn reply(&mut self, result: Result<TransportResponse, H3SendError>) {
        if let Some(tx) = self.resp_tx.take() {
            drop(tx.send(result));
        }
    }

    pub(super) fn deliver_head(&mut self) {
        if self.resp_tx.is_some() {
            let head = self.response(Vec::new());
            self.reply(Ok(head));
        }
        self.head_sent = true;
    }

    pub(super) fn deliver_request_body_error(&mut self, error: std::io::Error) {
        if self.head_sent {
            self.deliver_terminal(error);
        } else {
            self.reply(Err(H3SendError::RequestBody(error)));
        }
    }

    pub(super) fn deliver_error(
        &mut self,
        before_head: fn(String) -> H3SendError,
        message: String,
    ) {
        if self.head_sent {
            self.deliver_terminal(std::io::Error::other(message));
        } else {
            self.reply(Err(before_head(message)));
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
