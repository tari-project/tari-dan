//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use borsh::BorshSerialize;
use tari_ootle_common_types::{Epoch, NodeHeight};
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

impl ToSignatureMessage for TimeoutVote {
    fn to_signature_message(&self) -> FixedHash {
        TimeoutVoteMessage {
            epoch: self.epoch,
            height: self.height,
            high_pc_height: self.high_pc_height,
        }
        .to_signature_message()
    }
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

#[derive(BorshSerialize)]
pub struct TimeoutVoteMessage {
    pub epoch: Epoch,
    pub height: NodeHeight,
    pub high_pc_height: NodeHeight,
}

impl ToSignatureMessage for TimeoutVoteMessage {
    fn to_signature_message(&self) -> FixedHash {
        timeout_vote_signature_hasher()
            .chain(&self.epoch)
            .chain(&self.height)
            .chain(&self.high_pc_height)
            .finalize()
            .into()
    }
}
