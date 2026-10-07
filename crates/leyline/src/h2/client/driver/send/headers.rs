use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::h2::connection::{HeaderPair, encode_request_pseudos};
use crate::h2::error::H2Error;

const HPACK_ENTRY_OVERHEAD: usize = 32;
const OUTBOUND_HEADER_LIST: usize = crate::core::DEFAULT_MAX_HEADER_LIST_BYTES;

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
            .map(|(name, value)| name.as_ref().len() + value.as_ref().len() + HPACK_ENTRY_OVERHEAD)
            .sum();
        let size = pseudo_size + header_size;
        let limit = self
            .peer_settings
            .max_header_list_size
            .map_or(OUTBOUND_HEADER_LIST, |peer| {
                usize::try_from(peer)
                    .map_or(OUTBOUND_HEADER_LIST, |peer| peer.min(OUTBOUND_HEADER_LIST))
            });
        if size > limit {
            return Err(H2Error::Hpack(format!(
                "request header list of {size} bytes exceeds the peer's header list limit ({limit})"
            )));
        }
        Ok(())
    }

    pub(in crate::h2::client::driver) async fn write_headers_block(
        &mut self,
        stream_id: u32,
        end_stream: bool,
        (pseudo, headers): (&[(&str, &str)], &[HeaderPair]),
        with_priority: bool,
    ) -> Result<(), H2Error> {
        let fragment = encode_request_pseudos(&mut self.encoder, pseudo, headers);
        let max_frame = self.peer_settings.max_frame_size as usize;
        let priority = if with_priority {
            self.config.default_priority.map(|p| StreamDependency {
                exclusive: p.exclusive,
                dependency_id: p.stream_dependency,
                weight: p.weight,
            })
        } else {
            None
        };
        let prio_overhead = if priority.is_some() { 5 } else { 0 };

        if fragment.len() + prio_overhead <= max_frame {
            self.writer
                .write_headers(&HeadersFrame {
                    stream_id,
                    end_stream,
                    end_headers: true,
                    priority,
                    fragment: Bytes::from(fragment),
                })
                .await?;
        } else {
            let first_cap = max_frame.saturating_sub(prio_overhead).max(1);
            let first_len = first_cap.min(fragment.len());
            let first = &fragment[..first_len];
            self.writer
                .write_headers(&HeadersFrame {
                    stream_id,
                    end_stream,
                    end_headers: false,
                    priority,
                    fragment: Bytes::copy_from_slice(first),
                })
                .await?;
            self.write_continuations(stream_id, &fragment, first_len, max_frame)
                .await?;
        }
        Ok(())
    }

    pub(in crate::h2::client::driver) async fn write_continuations(
        &mut self,
        stream_id: u32,
        fragment: &[u8],
        mut offset: usize,
        max_frame: usize,
    ) -> Result<(), H2Error> {
        while offset < fragment.len() {
            let end = (offset + max_frame).min(fragment.len());
            let is_last = end == fragment.len();
            let chunk = &fragment[offset..end];
            let mut buf = BytesMut::with_capacity(9 + chunk.len());
            let header = crate::h2::frame::FrameHeader {
                length: chunk.len() as u32,
                frame_type: 0x9,
                flags: if is_last { 0x4 } else { 0 },
                stream_id,
            };
            header.encode(&mut buf);
            buf.extend_from_slice(chunk);
            self.writer.write_raw(&buf).await?;
            offset = end;
        }
        Ok(())
    }
}
