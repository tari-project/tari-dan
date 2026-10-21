//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{BufMut, Bytes, BytesMut};
use futures::{Stream, StreamExt};
use log::*;

#[derive(Debug, thiserror::Error)]
pub enum ProtobufStreamError {
    #[error("Reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("Prost decode error: {0}")]
    DecodeError(#[from] prost::DecodeError),
    #[error("Message size {len} exceeds maximum allowed size of {max} bytes")]
    MessageSizeExceeded { len: usize, max: usize },
    #[error("Stream ended with {remaining} bytes of an incomplete message")]
    TruncatedFrame { remaining: usize },
}

pub struct ProtobufStream<TMsg> {
    bytes_stream: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    buf: BytesMut,
    max_message_size: usize,
    _marker: std::marker::PhantomData<TMsg>,
}

impl<TMsg> ProtobufStream<TMsg> {
    pub fn new(bytes_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static) -> Self {
        Self::with_max_message_size(bytes_stream, 16 * 1024 * 1024) // Default to 16 MiB max message size
    }

    pub fn with_max_message_size(
        bytes_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
        max_message_size: usize,
    ) -> Self {
        Self {
            bytes_stream: Box::pin(bytes_stream.fuse()),
            buf: BytesMut::with_capacity(max_message_size),
            max_message_size,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<TMsg: prost::Message + Default> ProtobufStream<TMsg> {
    /// Decodes the first message in `buf` if the buffer holds all of it, leaving any following bytes buffered.
    fn try_decode_message(&mut self) -> Result<Option<TMsg>, ProtobufStreamError> {
        // A length-delimited varint is complete once a byte with MSB 0 is seen (max 10 bytes for u64).
        if self.buf.len() < 10 && self.buf.iter().all(|byte| byte & 0x80 != 0) {
            return Ok(None);
        }
        let len = prost::decode_length_delimiter(&self.buf[..])?;
        if len > self.max_message_size {
            return Err(ProtobufStreamError::MessageSizeExceeded {
                len,
                max: self.max_message_size,
            });
        }

        let frame_len = prost::length_delimiter_len(len) + len;
        if self.buf.len() < frame_len {
            trace!(
                "Buffering: have {} bytes, need {} bytes (including the length delimiter)",
                self.buf.len(),
                frame_len,
            );
            return Ok(None);
        }

        let msg = TMsg::decode_length_delimited(&mut self.buf)?;
        Ok(Some(msg))
    }
}

impl<TMsg: prost::Message + Default + Unpin> Stream for ProtobufStream<TMsg> {
    type Item = Result<TMsg, ProtobufStreamError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            // A single chunk may carry several messages, so drain the buffer before reading more.
            if let Some(result) = this.try_decode_message().transpose() {
                return Poll::Ready(Some(result));
            }

            match this.bytes_stream.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => this.buf.put(bytes),
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e.into()))),
                Poll::Ready(None) => {
                    if this.buf.is_empty() {
                        return Poll::Ready(None);
                    }
                    let remaining = this.buf.len();
                    this.buf.clear();
                    return Poll::Ready(Some(Err(ProtobufStreamError::TruncatedFrame { remaining })));
                },
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::TryStreamExt;
    use prost::Message;

    use super::*;
    use crate::protobuf::EndOfShard;

    fn frame(max_state_version: u64) -> Vec<u8> {
        EndOfShard { max_state_version }.encode_length_delimited_to_vec()
    }

    async fn decode_chunks(chunks: Vec<Vec<u8>>) -> Result<Vec<u64>, ProtobufStreamError> {
        let chunks = chunks.into_iter().map(|c| Ok(Bytes::from(c)));
        ProtobufStream::<EndOfShard>::new(futures::stream::iter(chunks))
            .map_ok(|msg| msg.max_state_version)
            .try_collect()
            .await
    }

    #[tokio::test]
    async fn it_decodes_one_frame_per_chunk() {
        let decoded = decode_chunks(vec![frame(1), frame(2), frame(3)]).await.unwrap();
        assert_eq!(decoded, [1, 2, 3]);
    }

    #[tokio::test]
    async fn it_decodes_every_frame_in_a_chunk() {
        let decoded = decode_chunks(vec![[frame(1), frame(2), frame(3)].concat(), frame(4)])
            .await
            .unwrap();
        assert_eq!(decoded, [1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn it_decodes_frames_split_across_chunks() {
        let bytes = [frame(1), frame(u64::MAX), frame(3)].concat();
        let chunks = bytes.chunks(1).map(<[u8]>::to_vec).collect();
        let decoded = decode_chunks(chunks).await.unwrap();
        assert_eq!(decoded, [1, u64::MAX, 3]);
    }

    #[tokio::test]
    async fn it_decodes_an_empty_message() {
        let decoded = decode_chunks(vec![[frame(0), frame(0)].concat()]).await.unwrap();
        assert_eq!(decoded, [0, 0]);
    }

    #[tokio::test]
    async fn it_errors_on_a_truncated_frame_at_end_of_stream() {
        let mut bytes = [frame(1), frame(2)].concat();
        bytes.pop();
        let mut stream = ProtobufStream::<EndOfShard>::new(futures::stream::iter([Ok(Bytes::from(bytes))]));
        assert_eq!(stream.next().await.unwrap().unwrap().max_state_version, 1);
        let err = stream.next().await.unwrap().unwrap_err();
        assert!(matches!(err, ProtobufStreamError::TruncatedFrame { .. }), "{err}");
        assert!(stream.next().await.is_none());
    }
}
