//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::RangeInclusive;

use tari_base_node_client::types::BaseLayerConsensusConstants;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_comms::types::CommsPublicKey;
use tari_core::{transactions::transaction_components::ValidatorNodeRegistration, ValidatorNodeBMT};
use tari_dan_common_types::{Epoch, ShardId};
use tokio::sync::{broadcast, oneshot};

use crate::{
    base_layer::error::EpochManagerError,
    traits::ShardCommitteeAllocation,
    validator_node::ValidatorNode,
    Committee,
};

type Reply<T> = oneshot::Sender<Result<T, EpochManagerError>>;

#[derive(Debug)]
pub enum EpochManagerRequest {
    CurrentEpoch {
        reply: Reply<Epoch>,
    },
    CurrentBlockHeight {
        reply: Reply<u64>,
    },
    GetValidatorNode {
        epoch: Epoch,
        addr: CommsPublicKey,
        reply: Reply<ValidatorNode<CommsPublicKey>>,
    },
    AddValidatorNodeRegistration {
        block_height: u64,
        registration: ValidatorNodeRegistration,
        reply: Reply<()>,
    },
    UpdateEpoch {
        block_height: u64,
        block_hash: FixedHash,
        reply: Reply<()>,
    },
    LastRegistrationEpoch {
        reply: Reply<Option<Epoch>>,
    },
    UpdateLastRegistrationEpoch {
        epoch: Epoch,
        reply: Reply<()>,
    },
    IsEpochValid {
        epoch: Epoch,
        reply: Reply<bool>,
    },
    GetCommittees {
        epoch: Epoch,
        shards: Vec<ShardId>,
        reply: Reply<Vec<ShardCommitteeAllocation<CommsPublicKey>>>,
    },
    GetCommittee {
        epoch: Epoch,
        shard: ShardId,
        reply: Reply<Committee<CommsPublicKey>>,
    },
    GetCommitteeForShardRange {
        epoch: Epoch,
        shard_range: RangeInclusive<ShardId>,
        reply: Reply<Committee<CommsPublicKey>>,
    },
    GetValidatorNodesPerEpoch {
        epoch: Epoch,
        reply: Reply<Vec<ValidatorNode<PublicKey>>>,
    },
    GetValidatorNodeBMT {
        epoch: Epoch,
        reply: Reply<ValidatorNodeBMT>,
    },
    GetValidatorNodeMerkleRoot {
        epoch: Epoch,
        reply: Reply<Vec<u8>>,
    },
    IsValidatorInCommitteeForCurrentEpoch {
        shard: ShardId,
        identity: PublicKey,
        reply: Reply<bool>,
    },
    FilterToLocalShards {
        epoch: Epoch,
        for_addr: PublicKey,
        available_shards: Vec<ShardId>,
        reply: Reply<Vec<ShardId>>,
    },
    Subscribe {
        reply: Reply<broadcast::Receiver<EpochManagerEvent>>,
    },
    NotifyScanningComplete {
        reply: Reply<()>,
    },
    RemainingRegistrationEpochs {
        reply: Reply<Option<Epoch>>,
    },
    GetBaseLayerConsensusConstants {
        reply: Reply<BaseLayerConsensusConstants>,
    },
    GetLocalShardRange {
        epoch: Epoch,
        for_addr: PublicKey,
        reply: Reply<RangeInclusive<ShardId>>,
    },
}

#[derive(Debug, Clone)]
pub enum EpochManagerEvent {
    EpochChanged(Epoch),
}

// -------------------------------- Conversions -------------------------------- //

use tari_dan_storage::global::models;
impl From<models::ValidatorNode> for ValidatorNode<PublicKey> {
    fn from(db_vn: models::ValidatorNode) -> Self {
        Self {
            shard_key: db_vn.shard_key,
            public_key: db_vn.public_key,
        }
    }
}
