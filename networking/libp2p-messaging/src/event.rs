//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use libp2p::PeerId;

use crate::{error::Error, stream::StreamId, MessageId};

#[derive(Debug)]
pub enum Event<TMsg> {
    ReceivedMessage {
        peer_id: PeerId,
        message: TMsg,
    },
    MessageSent {
        message_id: MessageId,
        stream_id: StreamId,
    },
    InboundFailure {
        peer_id: PeerId,
        stream_id: StreamId,
        error: Error,
    },
    OutboundFailure {
        peer_id: PeerId,
        stream_id: StreamId,
        error: Error,
    },
    StreamClosed {
        peer_id: PeerId,
        stream_id: StreamId,
    },
    InboundStreamClosed {
        peer_id: PeerId,
    },
    Error(Error),
}
