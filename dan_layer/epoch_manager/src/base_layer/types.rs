//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    ops::RangeInclusive,
};

use tari_base_node_client::types::BaseLayerConsensusConstants;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_core::transactions::transaction_components::ValidatorNodeRegistration;
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard},
    hashing::MergedValidatorNodeMerkleProof,
    shard_bucket::ShardBucket,
    Epoch,
    ShardId,
};
use tari_dan_storage::global::models::ValidatorNode;
use tokio::sync::{broadcast, oneshot};

use crate::{error::EpochManagerError, EpochManagerEvent};

type Reply<T> = oneshot::Sender<Result<T, EpochManagerError>>;

#[derive(Debug)]
pub enum EpochManagerRequest<TAddr> {
    CurrentEpoch {
        reply: Reply<Epoch>,
    },
    CurrentBlockHeight {
        reply: Reply<u64>,
    },
    GetValidatorNode {
        epoch: Epoch,
        addr: TAddr,
        reply: Reply<ValidatorNode<TAddr>>,
    },
    GetValidatorNodeByPublicKey {
        epoch: Epoch,
        public_key: PublicKey,
        reply: Reply<ValidatorNode<TAddr>>,
    },
    GetManyValidatorNodes {
        query: Vec<(Epoch, PublicKey)>,
        reply: Reply<HashMap<(Epoch, PublicKey), ValidatorNode<TAddr>>>,
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
        shards: HashSet<ShardId>,
        reply: Reply<HashMap<ShardBucket, Committee<TAddr>>>,
    },
    GetCommittee {
        epoch: Epoch,
        shard: ShardId,
        reply: Reply<Committee<TAddr>>,
    },
    GetCommitteeForShardRange {
        epoch: Epoch,
        shard_range: RangeInclusive<ShardId>,
        reply: Reply<Committee<TAddr>>,
    },
    GetValidatorNodesPerEpoch {
        epoch: Epoch,
        reply: Reply<Vec<ValidatorNode<TAddr>>>,
    },
    GetValidatorSetMergedMerkleProof {
        epoch: Epoch,
        validator_set: Vec<PublicKey>,
        reply: Reply<MergedValidatorNodeMerkleProof>,
    },
    GetValidatorNodeMerkleRoot {
        epoch: Epoch,
        reply: Reply<Vec<u8>>,
    },
    IsValidatorInCommitteeForCurrentEpoch {
        shard: ShardId,
        identity: TAddr,
        reply: Reply<bool>,
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
        for_addr: TAddr,
        reply: Reply<RangeInclusive<ShardId>>,
    },
    GetOurValidatorNode {
        epoch: Epoch,
        reply: Reply<ValidatorNode<TAddr>>,
    },
    GetCommitteeShard {
        epoch: Epoch,
        shard: ShardId,
        reply: Reply<CommitteeShard>,
    },
    GetLocalCommitteeShard {
        epoch: Epoch,
        reply: Reply<CommitteeShard>,
    },
    GetNumCommittees {
        epoch: Epoch,
        reply: Reply<u32>,
    },
    GetCommitteesByBuckets {
        epoch: Epoch,
        buckets: HashSet<ShardBucket>,
        reply: Reply<HashMap<ShardBucket, Committee<TAddr>>>,
    },
    GetFeeClaimPublicKey {
        reply: Reply<Option<PublicKey>>,
    },
    SetFeeClaimPublicKey {
        public_key: PublicKey,
        reply: Reply<()>,
    },
}
