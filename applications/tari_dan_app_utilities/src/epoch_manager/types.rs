//  Copyright 2023. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use tari_common_types::types::FixedHash;
use tari_comms::types::CommsPublicKey;
use tari_core::{transactions::transaction_components::ValidatorNodeRegistration, ValidatorNodeBMT};
use tari_dan_common_types::{Epoch, ShardId};
use tari_dan_core::{
    consensus_constants::{BaseLayerConsensusConstants, ConsensusConstants},
    models::{Committee, ValidatorNode},
    services::epoch_manager::{EpochManagerError, ShardCommitteeAllocation},
};
use tokio::sync::{broadcast, oneshot};

type Reply<T> = oneshot::Sender<Result<T, EpochManagerError>>;

#[derive(Debug)]
pub enum EpochManagerRequest {
    CurrentEpoch {
        reply: Reply<Epoch>,
    },
    CurrentBlockHeight {
        reply: Reply<u64>,
    },
    GetValidatorShardKey {
        epoch: Epoch,
        addr: CommsPublicKey,
        reply: Reply<ShardId>,
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
    GetValidatorNodesPerEpoch {
        epoch: Epoch,
        reply: Reply<Vec<ValidatorNode<CommsPublicKey>>>,
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
        identity: CommsPublicKey,
        reply: Reply<bool>,
    },
    FilterToLocalShards {
        epoch: Epoch,
        for_addr: CommsPublicKey,
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
    GetConsensusConstants {
        reply: Reply<ConsensusConstants>,
    },
}

#[derive(Debug, Clone)]
pub enum EpochManagerEvent {
    EpochChanged(Epoch),
}
