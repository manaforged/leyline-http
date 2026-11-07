use bytes::Bytes;

use super::{FrameHeader, PRIORITY_LEN, Scope, check_scope};
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;

pub(super) fn validate(header: &FrameHeader, payload: &Bytes) -> Result<(), H2Error> {
    check_scope(header, Scope::Stream, "PRIORITY")?;
    if payload.len() != PRIORITY_LEN {
        return Err(H2Error::Stream {
            stream_id: header.stream_id,
            code: ErrorCode::FrameSizeError,
        });
    }
    Ok(())
}
