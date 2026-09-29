use bytes::Bytes;

use super::FrameHeader;
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;

pub(super) fn validate(header: &FrameHeader, payload: &Bytes) -> Result<(), H2Error> {
    if header.stream_id == 0 {
        return Err(H2Error::Connection {
            code: ErrorCode::ProtocolError,
            reason: "PRIORITY on stream 0".into(),
        });
    }
    if payload.len() != 5 {
        return Err(H2Error::Connection {
            code: ErrorCode::FrameSizeError,
            reason: format!("PRIORITY must be 5 bytes, got {}", payload.len()),
        });
    }
    Ok(())
}
