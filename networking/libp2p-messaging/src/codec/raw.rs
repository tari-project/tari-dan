//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::io;

use async_trait::async_trait;
use bytes::Bytes;
use libp2p::futures::{AsyncRead, AsyncWrite};

use crate::codec::{Codec, read_frame, write_frame};

/// Carries each message as the raw bytes of its frame body, leaving its encoding to the caller.
///
/// An inbound message holds exactly the bytes it arrived as, so a receiver that queues messages before decoding
/// them can account for them precisely. The wire format matches [`ProstCodec`](crate::prost::ProstCodec)'s, so a
/// peer may use either.
#[derive(Debug, Clone, Copy, Default)]
pub struct BytesCodec;

#[async_trait]
impl Codec for BytesCodec {
    type Message = Bytes;

    async fn decode_from<R>(&self, reader: &mut R) -> io::Result<(usize, Self::Message)>
    where R: AsyncRead + Unpin + Send {
        let body = read_frame(reader).await?;
        Ok((body.len(), Bytes::from(body)))
    }

    async fn encode_to<W>(&self, writer: &mut W, message: Self::Message) -> io::Result<()>
    where W: AsyncWrite + Unpin + Send {
        write_frame(writer, &message).await
    }
}

#[cfg(test)]
mod tests {
    use libp2p::futures::executor::block_on;

    use super::*;
    use crate::codec::MAX_MESSAGE_SIZE;

    #[test]
    fn round_trips_a_message() {
        let message = Bytes::from(vec![7u8; 100_000]);
        let mut wire = Vec::new();
        block_on(BytesCodec.encode_to(&mut wire, message.clone())).unwrap();

        let (len, decoded) = block_on(BytesCodec.decode_from(&mut wire.as_slice())).unwrap();
        assert_eq!(len, message.len());
        assert_eq!(decoded, message);
    }

    #[test]
    fn a_frame_over_the_limit_is_refused_from_its_length_prefix() {
        let wire = ((MAX_MESSAGE_SIZE + 1) as u32).to_be_bytes();
        let err = block_on(BytesCodec.decode_from(&mut wire.as_slice())).unwrap_err();
        assert_eq!(err.to_string(), "message too large");

        let oversized = Bytes::from(vec![0u8; MAX_MESSAGE_SIZE + 1]);
        let err = block_on(BytesCodec.encode_to(&mut Vec::new(), oversized)).unwrap_err();
        assert_eq!(err.to_string(), "message too large");
    }

    #[cfg(feature = "prost")]
    #[test]
    fn reads_the_frames_prost_codec_writes() {
        use crate::prost::ProstCodec;

        let message = vec![7u8; 1000];
        let mut wire = Vec::new();
        block_on(ProstCodec::<Vec<u8>>::default().encode_to(&mut wire, message.clone())).unwrap();

        let (_, body) = block_on(BytesCodec.decode_from(&mut wire.as_slice())).unwrap();
        let decoded = <Vec<u8> as ::prost::Message>::decode(body).unwrap();
        assert_eq!(decoded, message);
    }
}
