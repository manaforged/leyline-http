use tokio::io::{AsyncRead, AsyncWrite};

use crate::h2::H2Error;
use crate::h2::connection::encode_request_pseudos;
use crate::h2::frame::PRIORITY_LEN;
use crate::h2::hpack::ENTRY_OVERHEAD as HPACK_ENTRY_OVERHEAD;
use crate::wire::DEFAULT_MAX_HEADER_LIST_BYTES;
use crate::wire::HeaderPair;

use super::*;

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(in crate::h2::client::driver) fn check_header_list(
        &self,
        pseudo: &[(&str, &str)],
        headers: &[HeaderPair],
    ) -> Result<(), H2Error> {
        let pseudo_size: usize = pseudo
            .iter()
            .map(|(name, value)| name.len() + value.len() + HPACK_ENTRY_OVERHEAD)
            .sum();
        let header_size: usize = headers
            .iter()
            .map(|(name, value)| name.len() + value.len() + HPACK_ENTRY_OVERHEAD)
            .sum();
        let size = pseudo_size + header_size;
        let limit =
            self.peer_settings
                .max_header_list_size
                .map_or(DEFAULT_MAX_HEADER_LIST_BYTES, |peer| {
                    usize::try_from(peer).map_or(DEFAULT_MAX_HEADER_LIST_BYTES, |peer| {
                        peer.min(DEFAULT_MAX_HEADER_LIST_BYTES)
                    })
                });
        if size > limit {
            return Err(H2Error::Hpack(format!(
                "request header list of {size} bytes exceeds the peer's header list limit ({limit})"
            )));
        }
        Ok(())
    }

    pub(in crate::h2::client::driver) fn write_headers_block(
        &mut self,
        stream_id: u32,
        end_stream: bool,
        (pseudo, headers): (&[(&str, &str)], &[HeaderPair]),
    ) {
        let max_frame = self.peer_settings.max_frame_size as usize;
        let priority = self.config.default_priority;
        let prio_overhead = if priority.is_some() { PRIORITY_LEN } else { 0 };
        let fragment = encode_request_pseudos(&mut self.encoder, pseudo, headers);

        if fragment.len() + prio_overhead <= max_frame {
            HeadersFrame {
                stream_id,
                end_stream,
                end_headers: true,
                priority,
                fragment,
            }
            .encode(self.writer.out());
        } else {
            let fragment = fragment.to_vec();
            let first_cap = max_frame.saturating_sub(prio_overhead).max(1);
            let first_len = first_cap.min(fragment.len());
            HeadersFrame {
                stream_id,
                end_stream,
                end_headers: false,
                priority,
                fragment: &fragment[..first_len],
            }
            .encode(self.writer.out());
            self.write_continuations(stream_id, &fragment, first_len, max_frame);
        }
    }

    fn write_continuations(
        &mut self,
        stream_id: u32,
        fragment: &[u8],
        mut offset: usize,
        max_frame: usize,
    ) {
        while offset < fragment.len() {
            let end = (offset + max_frame).min(fragment.len());
            ContinuationFrame {
                stream_id,
                end_headers: end == fragment.len(),
                fragment: &fragment[offset..end],
            }
            .encode(self.writer.out());
            offset = end;
        }
    }
}
