//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use tari_common_types::types::FixedHash;
use tari_ootle_common_types::{
    Epoch,
    ShardGroup,
    SubstateAddress,
    VotePower,
    committee::{Committee, CommitteeInfo},
};
use tari_ootle_storage::global::models::ValidatorNode;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
use tokio::sync::oneshot;

use crate::{error::EpochManagerError, service::NetworkDescription};

type Reply<T> = oneshot::Sender<Result<T, EpochManagerError>>;

#[derive(Debug)]
pub enum EpochManagerRequest<TAddr> {
    CurrentEpoch {
        reply: Reply<Epoch>,
    },
    GetEpochHash {
        epoch: Epoch,
        reply: Reply<FixedHash>,
    },
    GetCurrentEpochHash {
        reply: Reply<FixedHash>,
    },
    GetValidatorNodeByPublicKey {
        epoch: Epoch,
        public_key: RistrettoPublicKeyBytes,
        reply: Reply<ValidatorNode<TAddr>>,
    },
    AddValidatorNodeRegistration {
        activation_epoch: Epoch,
        validator_public_key: RistrettoPublicKeyBytes,
        claim_public_key: RistrettoPublicKeyBytes,
        power: VotePower,
        shard_key: SubstateAddress,
        reply: Reply<()>,
    },
    DeactivateValidatorNode {
        public_key: RistrettoPublicKeyBytes,
        deactivation_epoch: Epoch,
        reply: Reply<()>,
    },

    GetCommitteeForSubstate {
        epoch: Epoch,
        substate_address: SubstateAddress,
        reply: Reply<Arc<Committee<TAddr>>>,
    },
    GetCommitteeInfoByAddress {
        epoch: Epoch,
        address: TAddr,
        reply: Reply<CommitteeInfo>,
    },
    GetValidatorNodesPerEpoch {
        epoch: Epoch,
        reply: Reply<Vec<ValidatorNode<TAddr>>>,
    },
    WaitForInitialScanningToComplete {
        reply: Reply<()>,
    },
    IsInitialScanningComplete {
        reply: Reply<bool>,
    },
    GetOurValidatorNode {
        epoch: Epoch,
        reply: Reply<ValidatorNode<TAddr>>,
    },
    GetCommitteeInfoForSubstate {
        epoch: Epoch,
        substate_address: SubstateAddress,
        reply: Reply<CommitteeInfo>,
    },
    GetLocalCommitteeInfo {
        epoch: Epoch,
        reply: Reply<CommitteeInfo>,
    },
    GetCommitteeInfo {
        epoch: Epoch,
        shard_group: ShardGroup,
        reply: Reply<CommitteeInfo>,
    },
    GetNumCommittees {
        epoch: Epoch,
        reply: Reply<u32>,
    },
    GetCommitteeForShardGroup {
        epoch: Epoch,
        shard_group: ShardGroup,
        reply: Reply<Arc<Committee<TAddr>>>,
    },
    GetCommitteesOverlappingShardGroup {
        epoch: Epoch,
        shard_group: ShardGroup,
        reply: Reply<HashMap<ShardGroup, Committee<TAddr>>>,
    },
    GetFeeClaimPublicKey {
        reply: Reply<Option<RistrettoPublicKeyBytes>>,
    },
    GetRandomCommitteeMemberFromShardGroup {
        epoch: Epoch,
        shard_group: Option<ShardGroup>,
        excluding: HashSet<TAddr>,
        reply: Reply<ValidatorNode<TAddr>>,
    },
    GetNetworkDescription {
        reply: Reply<NetworkDescription>,
    },
    LockEpoch {
        epoch: Epoch,
        reply: Reply<()>,
    },
    GetObservedEpochHash {
        epoch: Epoch,
        reply: Reply<Option<FixedHash>>,
    },
    GetBirthdayEpoch {
        reply: Reply<Option<Epoch>>,
    },
}
