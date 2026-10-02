use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::h2::error::H2Error;

use super::*;

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(in crate::h2::client::driver) async fn write_headers_block(
        &mut self,
        stream_id: u32,
        end_stream: bool,
        fragment: Vec<u8>,
        with_priority: bool,
    ) -> Result<(), H2Error> {
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
