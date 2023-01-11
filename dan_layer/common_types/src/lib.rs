// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt,
    fmt::{Display, Formatter},
};

use ::serde::{Deserialize, Serialize};
use tari_bor::{borsh, Decode, Encode};
use tari_common_types::types::{FixedHash, FixedHashSizeError};
use tari_engine_types::substate::Substate;
use tari_utilities::hex::Hex;

pub mod proto;

mod epoch;
pub use epoch::Epoch;

pub mod hashing;
pub mod optional;
pub mod serde_with;

pub mod quorum_certificate;
pub use quorum_certificate::{QuorumCertificate, QuorumDecision, QuorumRejectReason};

mod node_height;
pub use node_height::NodeHeight;

mod shard_vote;
pub use shard_vote::ShardPledge;

mod tree_node_hash;
pub use tree_node_hash::TreeNodeHash;

mod validator_metadata;
pub use validator_metadata::{vn_mmr_node_hash, ValidatorMetadata};

mod object_pledge;
pub use object_pledge::{ObjectPledge, ObjectPledgeInfo};

mod node_addressable;
pub use node_addressable::NodeAddressable;

mod shard_id;
pub use shard_id::ShardId;

#[derive(Clone, Debug, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum SubstateChange {
    /// An "Up" state
    Create,
    /// Substate exists but will not be created/destroyed
    Exists,
    /// A "Down" state
    Destroy,
}

impl Display for SubstateChange {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            SubstateChange::Create => write!(f, "Create"),
            SubstateChange::Exists => write!(f, "Exists"),
            SubstateChange::Destroy => write!(f, "Destroy"),
        }
    }
}

#[derive(Debug, Clone, Encode, Decode, Deserialize, Serialize)]
pub enum SubstateState {
    DoesNotExist,
    Up { created_by: PayloadId, data: Substate },
    Down { deleted_by: PayloadId },
}

impl SubstateState {
    pub fn as_str(&self) -> &str {
        match self {
            Self::DoesNotExist => "DoesNotExist",
            Self::Up { .. } => "Up",
            Self::Down { .. } => "Down",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ObjectClaim {}

impl ObjectClaim {
    pub fn is_valid(&self, _payload: PayloadId) -> bool {
        // TODO: Implement this
        true
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Encode, Decode, Deserialize, Serialize)]
pub struct PayloadId {
    #[serde(with = "serde_with::hex")]
    id: [u8; 32],
}

impl PayloadId {
    pub fn new<T: AsRef<[u8]>>(id: T) -> Self {
        let mut v = [0u8; 32];
        assert_eq!(id.as_ref().len(), 32);
        v.copy_from_slice(id.as_ref());
        Self { id: v }
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.id.as_slice()
    }

    pub fn into_array(self) -> [u8; 32] {
        self.id
    }
}

impl Display for PayloadId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id.to_hex())
    }
}

impl TryFrom<Vec<u8>> for PayloadId {
    type Error = FixedHashSizeError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        Self::try_from(value.as_slice())
    }
}

impl TryFrom<&[u8]> for PayloadId {
    type Error = FixedHashSizeError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Ok(PayloadId::new(FixedHash::try_from(value)?))
    }
}
