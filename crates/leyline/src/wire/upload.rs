use std::sync::Arc;

use tokio::sync::{Semaphore, mpsc};
use tokio::task::AbortHandle;

use super::BoxedStream;
use crate::util::upload::{BodyChunk, pump_request_body, upload_credit};

/// The task that feeds one stream's request body, aborted when dropped.
pub(crate) struct Upload {
    credit: Arc<Semaphore>,
    pump: AbortHandle,
}

impl Upload {
    pub(crate) fn start<Id>(
        stream_id: Id,
        body: BoxedStream,
        tx: mpsc::Sender<BodyChunk<Id>>,
    ) -> Self
    where
        Id: Copy + Send + 'static,
    {
        let credit = upload_credit();
        let pump = tokio::spawn(pump_request_body(stream_id, body, tx, Arc::clone(&credit)));
        Self {
            credit,
            pump: pump.abort_handle(),
        }
    }

    pub(crate) fn sent(&self, bytes: usize) {
        self.credit.add_permits(bytes);
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        self.pump.abort();
    }
}
