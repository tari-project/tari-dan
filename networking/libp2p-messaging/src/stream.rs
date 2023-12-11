//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use libp2p::{
    futures::{channel::mpsc, SinkExt, Stream, StreamExt},
    PeerId,
};

pub type StreamId = u64;
pub fn channel<T>(stream_id: StreamId, peer_id: PeerId, size: usize) -> (MessageSink<T>, MessageStream<T>) {
    let (sender, receiver) = mpsc::channel(size);
    let sink = MessageSink::new(stream_id, peer_id, sender);
    let stream = MessageStream::new(stream_id, peer_id, receiver);
    (sink, stream)
}

#[derive(Debug)]
pub struct MessageStream<TMsg> {
    stream_id: StreamId,
    peer_id: PeerId,
    receiver: mpsc::Receiver<TMsg>,
}

impl<TMsg> MessageStream<TMsg> {
    pub fn new(stream_id: StreamId, peer_id: PeerId, receiver: mpsc::Receiver<TMsg>) -> Self {
        Self {
            stream_id,
            peer_id,
            receiver,
        }
    }

    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    pub async fn recv(&mut self) -> Option<TMsg> {
        self.receiver.next().await
    }
}

pub struct MessageSink<TMsg> {
    stream_id: StreamId,
    peer_id: PeerId,
    sender: mpsc::Sender<TMsg>,
}

impl<TMsg> MessageSink<TMsg> {
    pub fn new(stream_id: StreamId, peer_id: PeerId, sender: mpsc::Sender<TMsg>) -> Self {
        Self {
            stream_id,
            peer_id,
            sender,
        }
    }

    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    pub async fn send(&mut self, msg: TMsg) -> Result<(), crate::Error> {
        self.sender.send(msg).await.map_err(|_| crate::Error::ChannelClosed)
    }

    pub async fn send_all<TStream>(&mut self, stream: &mut TStream) -> Result<(), crate::Error>
    where TStream: Stream<Item = Result<TMsg, mpsc::SendError>> + Unpin + ?Sized {
        self.sender
            .send_all(stream)
            .await
            .map_err(|_| crate::Error::ChannelClosed)
    }
}
