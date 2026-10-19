//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{AsyncRead, AsyncWrite};
use tokio::time::{self, Sleep};

/// A substream whose writes fail once the peer has taken nothing written to it for `timeout`. The time runs from
/// the first write that cannot go ahead to the next one that does, so a peer that reads slowly but steadily is
/// unaffected. Once a write has timed out every later one fails at once: the frame it belonged to was cut short,
/// so nothing written after it could be read.
pub struct WriteTimeout<T> {
    inner: T,
    timeout: Duration,
    stalled_since: Option<Pin<Box<Sleep>>>,
    timed_out: bool,
}

impl<T: AsyncWrite + Unpin> WriteTimeout<T> {
    pub fn new(inner: T, timeout: Duration) -> Self {
        Self {
            inner,
            timeout,
            stalled_since: None,
            timed_out: false,
        }
    }

    fn poll_write_op<R>(
        &mut self,
        cx: &mut Context<'_>,
        op: impl FnOnce(Pin<&mut T>, &mut Context<'_>) -> Poll<io::Result<R>>,
    ) -> Poll<io::Result<R>> {
        if self.timed_out {
            return Poll::Ready(Err(self.timed_out_error()));
        }
        if let Poll::Ready(result) = op(Pin::new(&mut self.inner), cx) {
            self.stalled_since = None;
            return Poll::Ready(result);
        }
        let timeout = self.timeout;
        let stall = self.stalled_since.get_or_insert_with(|| Box::pin(time::sleep(timeout)));
        if stall.as_mut().poll(cx).is_pending() {
            return Poll::Pending;
        }
        self.stalled_since = None;
        self.timed_out = true;
        Poll::Ready(Err(self.timed_out_error()))
    }

    fn timed_out_error(&self) -> io::Error {
        io::Error::new(
            io::ErrorKind::TimedOut,
            format!("the peer took nothing written to it for {:.0?}", self.timeout),
        )
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for WriteTimeout<T> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for WriteTimeout<T> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        self.poll_write_op(cx, |inner, cx| inner.poll_write(cx, buf))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_write_op(cx, |inner, cx| inner.poll_flush(cx))
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_write_op(cx, |inner, cx| inner.poll_close(cx))
    }
}

#[cfg(test)]
mod tests {
    use futures::{AsyncReadExt, AsyncWriteExt};
    use tokio_util::compat::TokioAsyncReadCompatExt;

    use super::*;

    #[tokio::test]
    async fn a_write_the_peer_takes_nothing_of_fails_at_the_timeout_and_so_does_every_later_one() {
        let (local, _peer) = tokio::io::duplex(4);
        let mut stream = WriteTimeout::new(local.compat(), Duration::from_millis(100));

        let err = time::timeout(Duration::from_secs(5), stream.write_all(b"more than the window"))
            .await
            .expect("the stalled write was never cut short")
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);

        let err = stream.write(b"x").await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        let err = stream.close().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn a_peer_reading_steadily_is_given_as_long_as_it_takes() {
        let (local, peer) = tokio::io::duplex(4);
        let mut stream = WriteTimeout::new(local.compat(), Duration::from_secs(1));
        let payload = vec![7u8; 128];

        // Each read frees the window well within the timeout, but the whole payload takes longer than it to go.
        let reader = tokio::spawn(async move {
            let mut peer = peer.compat();
            let mut received = Vec::new();
            let mut buf = [0u8; 4];
            while received.len() < 128 {
                time::sleep(Duration::from_millis(50)).await;
                let n = peer.read(&mut buf).await.unwrap();
                received.extend_from_slice(&buf[..n]);
            }
            received
        });

        stream.write_all(&payload).await.unwrap();
        stream.flush().await.unwrap();
        assert_eq!(reader.await.unwrap(), payload);
    }
}
