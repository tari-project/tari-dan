//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use libp2p::{PeerId, gossipsub};
use tari_swarm::messaging::{Codec, prost::ProstCodec};

use crate::{TariMessage, proto};

/// The largest message the gossip topics accept.
///
/// Every node on the transaction mesh must agree on this, for the same reason they must agree on
/// [`TRANSACTION_TOPIC`]: a node with a smaller limit rejects messages its peers consider valid, and
/// does so as a codec frame error rather than a per-message drop.
///
/// The consensus topic does not bear on the figure. A foreign proposal's substate bundle is
/// requested and answered over the messaging protocol; the topic carries only the notification that
/// one exists.
///
/// # This limit is below what ingress admits
///
/// The transaction topic sets the figure, and the largest transaction ingress accepts is bounded by
/// `ConsensusConstants::max_transaction_weight`, not by any byte cap. Blob payloads are charged at
/// `calc_blobs_weight`'s divisor and `Blobs` is a transaction-level list, so a transaction may carry
/// a maximum-size template binary *and* further blob arguments and still weigh under the cap —
/// roughly 2.8 MiB of payload against this 2 MiB. Such a transaction validates everywhere and
/// gossips nowhere.
///
/// Closing the gap means either raising this to what the weight cap admits, or bounding transaction
/// bytes at ingress so the weight cap stops being the only limit. The second is the better shape and
/// the more disruptive change: it makes transactions invalid that are valid today, so it belongs
/// with a protocol activation rather than in a constant.
pub const MAX_GOSSIP_MESSAGE_SIZE: usize = 2 * 1024 * 1024;

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
