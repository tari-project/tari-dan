//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use libp2p::{PeerId, gossipsub};
use tari_swarm::messaging::{Codec, prost::ProstCodec};

use crate::{TariMessage, proto};

/// Room above `max_transaction_size_bytes` for the protobuf wrapper the transaction travels in: the
/// `TariMessage` envelope, the `Transaction.bor_encoded` field tag and its length prefix.
///
/// Loose against those — they are tens of bytes — because the cost of being wrong is asymmetric: a
/// few spare kibibytes per message against refusing an admissible transaction at the frame boundary,
/// which surfaces as a codec error and costs the sender's peer score.
const GOSSIP_FRAMING_ALLOWANCE: usize = 16 * 1024;

/// The largest message the gossip topics accept, for a network with the given transaction byte cap.
///
/// The transaction topic sets the figure: a transaction admitted at ingress must be one the mesh can
/// carry, so this is `ConsensusConstants::max_transaction_size_bytes` plus the framing it travels in.
/// Deriving it keeps the two from drifting — a limit below what ingress admits refuses valid
/// transactions, and does so as a codec frame error rather than a per-message drop, because the
/// per-topic size map gossipsub checks messages against is not populated here.
///
/// The consensus topic does not bear on the figure. A foreign proposal's substate bundle is requested
/// and answered over the messaging protocol; the topic carries only the notification that one exists.
///
/// Every node on a network must agree on the result, for the same reason they must agree on
/// [`TRANSACTION_TOPIC`]: a node with a smaller limit rejects messages its peers consider valid.
///
/// # Narrowing this is a rollout decision
///
/// The invariant is one-directional — gossip must carry anything ingress admits, and nothing
/// requires it to be tight — so raising the transaction byte cap is always safe to deploy in any
/// order, while lowering it is not. Between a node on the lower limit and a peer still on the higher
/// one there is a band of transactions the peer admits and relays whose frame the upgraded node
/// cannot decode, which costs the relaying peer a torn-down substream rather than a per-message
/// drop.
///
/// The current value is below the flat 2 MiB it replaces, which is deliberate and safe here only
/// because every validator and indexer on these networks is upgraded together. A deployment that
/// cannot do that must floor the result at the limit its peers already run, or raise the cap first
/// and narrow it in a later release.
pub const fn max_gossip_message_size(max_transaction_size_bytes: usize) -> usize {
    max_transaction_size_bytes + GOSSIP_FRAMING_ALLOWANCE
}

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
