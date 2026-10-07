use std::io;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use tokio::sync::{Semaphore, mpsc};

pub(crate) type BodyStream = Pin<Box<dyn Stream<Item = io::Result<Bytes>> + Send + 'static>>;

pub(crate) const UPLOAD_WINDOW: usize = 256 * 1024;

pub(crate) const UPLOAD_CHUNK: usize = 16 * 1024;

pub(crate) enum BodyChunk<Id> {
    Chunk {
        stream_id: Id,
        data: Bytes,
    },
    Eof {
        stream_id: Id,
        error: Option<io::Error>,
    },
}

pub(crate) fn upload_credit() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(UPLOAD_WINDOW))
}

pub(crate) async fn pump_request_body<Id, S>(
    stream_id: Id,
    mut body: S,
    tx: mpsc::Sender<BodyChunk<Id>>,
    credit: Arc<Semaphore>,
) where
    Id: Copy,
    S: Stream<Item = io::Result<Bytes>> + Unpin,
{
    while let Some(item) = body.next().await {
        match item {
            Ok(data) if data.is_empty() => {
                if tx.is_closed() {
                    return;
                }
                tokio::task::yield_now().await;
            }
            Ok(mut data) => {
                while !data.is_empty() {
                    let take = data.len().min(UPLOAD_CHUNK);
                    let slice = data.split_to(take);
                    let Ok(permit) = credit.acquire_many(take as u32).await else {
                        return;
                    };
                    permit.forget();
                    if tx
                        .send(BodyChunk::Chunk {
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
                    tx.send(BodyChunk::Eof {
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
        tx.send(BodyChunk::Eof {
            stream_id,
            error: None,
        })
        .await,
    );
}
