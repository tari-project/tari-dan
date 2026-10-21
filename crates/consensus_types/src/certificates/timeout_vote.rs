//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use ootle_network::Network;
use tari_ootle_common_types::{Epoch, NodeHeight, ProtocolVersion};
use tari_sidechain::{FixedHash, QuorumDecision, hashing::timeout_vote_signature_hasher};
use tari_template_lib::types::crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes};

use crate::{SignedMessage, SignedTimeout, ToSignatureMessage, Vote, validator_signature::ValidatorSignatureBytes};

#[derive(Debug, Clone, serde::Serialize)]
pub struct TimeoutVote {
    pub epoch: Epoch,
    pub height: NodeHeight,
    /// The height of the signer's high proposal certificate. The leader of the next view must justify from a
    /// certificate at least as high as the highest one a quorum attests to here.
    pub high_pc_height: NodeHeight,
    pub signature: ValidatorSignatureBytes,
}

impl TimeoutVote {
    pub fn signature(&self) -> &ValidatorSignatureBytes {
        &self.signature
    }

    /// The message this vote signs as a validator of `network`.
    pub fn to_message(&self, network: Network) -> TimeoutVoteMessage {
        TimeoutVoteMessage::new(network, self.epoch, self.height, self.high_pc_height)
    }

    pub fn as_signed_timeout(&self) -> SignedTimeout {
        SignedTimeout {
            high_pc_height: self.high_pc_height,
            signature: self.signature.clone(),
        }
    }
}

impl Vote for TimeoutVote {
    type AggregationKey = ();

    fn epoch(&self) -> Epoch {
        self.epoch
    }

    fn height(&self) -> NodeHeight {
        self.height
    }

    fn decision(&self) -> QuorumDecision {
        QuorumDecision::Accept
    }

    fn aggregation_key(&self) {}
}

impl SignedMessage for TimeoutVote {
    fn signature(&self) -> &SchnorrSignatureBytes {
        &self.signature.signature
    }

    fn public_key(&self) -> &RistrettoPublicKeyBytes {
        &self.signature.public_key
    }
}

impl Display for TimeoutVote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TimeoutVote {{ epoch: {}, height: {}, high pc height: {}, signer: {} }}",
            self.epoch, self.height, self.high_pc_height, self.signature.public_key
        )
    }
}

/// The message a timeout vote signs. The vote carries neither the network nor the protocol version, so the signer and
/// every verifier supply the network they run on, and [`Self::new`] resolves the version from that network's schedule.
#[derive(Debug, Clone)]
pub struct TimeoutVoteMessage {
    network: Network,
    protocol_version: ProtocolVersion,
    epoch: Epoch,
    height: NodeHeight,
    high_pc_height: NodeHeight,
}

impl TimeoutVoteMessage {
    pub fn new(network: Network, epoch: Epoch, height: NodeHeight, high_pc_height: NodeHeight) -> Self {
        Self {
            network,
            protocol_version: ProtocolVersion::at(network, epoch),
            epoch,
            height,
            high_pc_height,
        }
    }
}

impl ToSignatureMessage for TimeoutVoteMessage {
    fn to_signature_message(&self) -> FixedHash {
        timeout_vote_signature_hasher()
            .chain(&self.network.as_byte())
            .chain(&self.protocol_version.as_u32())
            .chain(&self.epoch)
            .chain(&self.height)
            .chain(&self.high_pc_height)
            .finalize()
            .into()
    }
}
