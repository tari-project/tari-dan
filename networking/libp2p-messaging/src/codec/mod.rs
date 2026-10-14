//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#[cfg(feature = "prost")]
pub mod prost;
mod raw;

use std::{fmt, io};

use libp2p::futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
pub use raw::BytesCodec;

/// The most bytes one message may occupy on the wire, excluding its length prefix.
pub const MAX_MESSAGE_SIZE: usize = 4 * 1024 * 1024;
const INITIAL_READ_CAPACITY: usize = 64 * 1024;

/// A `Codec` defines the request and response types
/// for a request-response [`Behaviour`](crate::Behaviour) protocol or
/// protocol family and how they are encoded / decoded on an I/O stream.
#[async_trait::async_trait]
pub trait Codec: Default {
    /// The type of inbound and outbound message.
    type Message: fmt::Debug + Send;

    /// Reads a message from the given I/O stream according to the
    /// negotiated protocol.
    async fn decode_from<R>(&self, reader: &mut R) -> io::Result<(usize, Self::Message)>
    where R: AsyncRead + Unpin + Send;

    /// Writes a request to the given I/O stream according to the
    /// negotiated protocol.
    async fn encode_to<W>(&self, writer: &mut W, message: Self::Message) -> io::Result<()>
    where W: AsyncWrite + Unpin + Send;
}

/// Reads one length-prefixed frame of at most [`MAX_MESSAGE_SIZE`] bytes and returns its body.
async fn read_frame<R>(reader: &mut R) -> io::Result<Vec<u8>>
where R: AsyncRead + Unpin {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_MESSAGE_SIZE {
        return Err(io::Error::other("message too large"));
    }
    read_exact_growing(reader, len).await
}

/// Writes `body` as one length-prefixed frame.
async fn write_frame<W>(writer: &mut W, body: &[u8]) -> io::Result<()>
where W: AsyncWrite + Unpin {
    let len = body.len();
    if len > MAX_MESSAGE_SIZE {
        return Err(io::Error::other("message too large"));
    }
    writer.write_all(&(len as u32).to_be_bytes()).await?;
    writer.write_all(body).await?;
    Ok(())
}

/// Reads exactly `len` bytes. The buffer grows as bytes arrive: each step at most doubles what has been received and
/// never exceeds `len`.
async fn read_exact_growing<R>(reader: &mut R, len: usize) -> io::Result<Vec<u8>>
where R: AsyncRead + Unpin {
    let mut buf = Vec::new();
    while buf.len() < len {
        let start = buf.len();
        let end = len.min(start.saturating_mul(2).max(INITIAL_READ_CAPACITY));
        buf.reserve_exact(end - start);
        buf.resize(end, 0);
        reader.read_exact(&mut buf[start..]).await?;
    }
    Ok(buf)
}
