//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
};

use tari_base_node_client::types::BaseLayerConsensusConstants;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_core::transactions::{tari_amount::MicroMinotari, transaction_components::ValidatorNodeRegistration};
use tari_dan_common_types::{
    committee::{Committee, CommitteeInfo, NetworkCommitteeInfo},
    shard::Shard,
    Epoch,
    SubstateAddress,
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
    CurrentBlockInfo {
        reply: Reply<(u64, FixedHash)>,
    },
    GetLastBlockOfTheEpoch {
        reply: Reply<FixedHash>,
    },
    IsLastBlockOfTheEpoch {
        block_height: u64,
        reply: Reply<bool>,
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
        value: MicroMinotari,
        reply: Reply<()>,
    },
    AddBlockHash {
        block_height: u64,
        block_hash: FixedHash,
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
        reply: Reply<HashMap<Shard, Committee<TAddr>>>,
    },
    GetCommitteeForSubstate {
        epoch: Epoch,
        substate_address: SubstateAddress,
        reply: Reply<Committee<TAddr>>,
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
    GetOurValidatorNode {
        epoch: Epoch,
        reply: Reply<ValidatorNode<TAddr>>,
    },
    GetCommitteeInfo {
        epoch: Epoch,
        substate_address: SubstateAddress,
        reply: Reply<CommitteeInfo>,
    },
    GetLocalCommitteeInfo {
        epoch: Epoch,
        reply: Reply<CommitteeInfo>,
    },
    GetNumCommittees {
        epoch: Epoch,
        reply: Reply<u32>,
    },
    GetCommitteesForShards {
        epoch: Epoch,
        shards: HashSet<Shard>,
        reply: Reply<HashMap<Shard, Committee<TAddr>>>,
    },
    GetBaseLayerBlockHeight {
        hash: FixedHash,
        reply: Reply<Option<u64>>,
    },
    GetFeeClaimPublicKey {
        reply: Reply<Option<PublicKey>>,
    },
    SetFeeClaimPublicKey {
        public_key: PublicKey,
        reply: Reply<()>,
    },
    GetNetworkCommittees {
        reply: Reply<NetworkCommitteeInfo<TAddr>>,
    },
}
