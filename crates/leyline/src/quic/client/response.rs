use bytes::Bytes;
use leyline_quiche as quiche;
use quiche::h3::NameValue;

use crate::wire::{BlockReader, ResponseBlock, ResponseState};

pub(super) fn response_block(
    state: &mut ResponseState,
    list: &[quiche::h3::Header],
) -> Result<ResponseBlock, &'static str> {
    let mut block = BlockReader::new(*state);
    for header in list {
        block.field(header.name(), header.value(), || {
            http::HeaderName::from_bytes(header.name()).ok().zip(
                http::HeaderValue::from_maybe_shared(Bytes::copy_from_slice(header.value())).ok(),
            )
        });
    }
    block.finish(state)
}
