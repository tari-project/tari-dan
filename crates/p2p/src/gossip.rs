//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use libp2p::{PeerId, gossipsub};
use tari_engine_types::limits::ENGINE_LIMITS;
use tari_swarm::messaging::{Codec, prost::ProstCodec};

use crate::{TariMessage, proto};

/// Room above a template binary for the rest of the transaction carrying it: its other instructions,
/// inputs, signatures and protobuf framing. Generous against those — a single instruction is capped
/// at `ENGINE_LIMITS.max_call_size` — because a legitimate transaction refused at ingress is a worse
/// failure than the memory a larger allowance costs.
const TRANSACTION_ENVELOPE_ALLOWANCE: usize = 256 * 1024;

/// The largest message the gossip topics accept.
///
/// The transaction topic sets this. A publishing transaction carries a WASM binary of up to
/// `ENGINE_LIMITS.max_template_binary_size_bytes`, and `MAX_PUBLISH_TEMPLATES_PER_TRANSACTION` holds
/// it to one such binary, so that binary plus the transaction around it is the largest thing that can
/// legitimately be gossiped. Deriving it here rather than choosing a round number keeps the two moving
/// together: a template limit that shrinks shrinks this with it.
///
/// The consensus topic does not enter into it. A foreign proposal's substate bundle is requested and
/// answered over the messaging protocol; the topic carries only the notification that one exists.
///
/// Every node on the transaction mesh must agree on this, for the same reason they must agree on
/// [`TRANSACTION_TOPIC`]: a node with a smaller limit silently rejects messages its peers consider
/// valid.
pub const MAX_GOSSIP_MESSAGE_SIZE: usize =
    ENGINE_LIMITS.max_template_binary_size_bytes + TRANSACTION_ENVELOPE_ALLOWANCE;

/// All transactions are gossiped on a single network-wide topic. Using one topic (rather than a topic per shard group)
/// keeps the gossipsub mesh stable across epoch boundaries, since validators never need to unsubscribe and resubscribe
/// when they are shuffled into a different shard group.
///
/// Shared by every node that participates in the transaction mesh — validators and indexers alike. A second definition
/// that drifted from this one would silently partition its holder from the mesh, with no error anywhere.
pub const TRANSACTION_TOPIC: &str = "transactions";

pub fn transaction_topic() -> String {
    TRANSACTION_TOPIC.to_string()
}

/// Wire codec for [`TariMessage`] as carried on the transaction gossip topic.
#[derive(Debug, Default)]
pub struct TransactionGossipCodec {
    codec: ProstCodec<proto::network::TariMessage>,
}

impl TransactionGossipCodec {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn encode(&self, message: TariMessage) -> std::io::Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(1024);
        let message = proto::network::TariMessage::from(&message);
        self.codec.encode_to(&mut buf, message).await?;
        Ok(buf)
    }

    pub async fn decode(&self, message: gossipsub::Message) -> std::io::Result<(usize, TariMessage)> {
        let (length, message) = self.codec.decode_from(&mut message.data.as_slice()).await?;
        let message = TariMessage::try_from(message).map_err(std::io::Error::other)?;

        Ok((length, message))
    }
}

/// Handle identifying one inbound gossip message for the purpose of reporting its validation
/// verdict. Deliberately not `Clone`: a verdict is reported once per message.
#[derive(Debug)]
pub struct GossipValidation {
    key: (gossipsub::MessageId, PeerId),
}

impl GossipValidation {
    pub fn new(key: (gossipsub::MessageId, PeerId)) -> Self {
        Self { key }
    }

    pub fn into_key(self) -> (gossipsub::MessageId, PeerId) {
        self.key
    }
}
