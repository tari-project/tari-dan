//  Copyright 2022. The Tari Project
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

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_dan_common_types::{
    committee::{Committee, CommitteeInfo},
    shard::Shard,
    Epoch,
    NodeAddressable,
    SubstateAddress,
};
use tari_dan_storage::global::models::ValidatorNode;
use tokio::sync::broadcast;

use crate::{EpochManagerError, EpochManagerEvent};

#[async_trait]
pub trait EpochManagerReader: Send + Sync {
    type Addr: NodeAddressable;

    async fn subscribe(&self) -> Result<broadcast::Receiver<EpochManagerEvent>, EpochManagerError>;

    async fn wait_for_initial_scanning_to_complete(&self) -> Result<(), EpochManagerError>;

    async fn get_all_validator_nodes(&self, epoch: Epoch) -> Result<Vec<ValidatorNode<Self::Addr>>, EpochManagerError>;

    async fn get_committees(&self, epoch: Epoch) -> Result<HashMap<Shard, Committee<Self::Addr>>, EpochManagerError>;
    async fn get_committee_info_by_validator_address(
        &self,
        epoch: Epoch,
        address: &Self::Addr,
    ) -> Result<CommitteeInfo, EpochManagerError>;
    async fn get_committee_for_substate(
        &self,
        epoch: Epoch,
        substate_address: SubstateAddress,
    ) -> Result<Committee<Self::Addr>, EpochManagerError>;

    async fn get_validator_node(
        &self,
        epoch: Epoch,
        addr: &Self::Addr,
    ) -> Result<ValidatorNode<Self::Addr>, EpochManagerError>;

    async fn get_validator_node_by_public_key(
        &self,
        epoch: Epoch,
        public_key: &PublicKey,
    ) -> Result<ValidatorNode<Self::Addr>, EpochManagerError>;

    /// Returns a list of validator nodes with the given epoch and public key. If any validator node is not found, an
    /// error is returned.
    async fn get_many_validator_nodes(
        &self,
        query: Vec<(Epoch, PublicKey)>,
    ) -> Result<HashMap<(Epoch, PublicKey), ValidatorNode<Self::Addr>>, EpochManagerError> {
        #[allow(clippy::mutable_key_type)]
        let mut results = HashMap::with_capacity(query.len());
        for (epoch, public_key) in query {
            let vn = self.get_validator_node_by_public_key(epoch, &public_key).await?;
            results.insert((epoch, public_key), vn);
        }
        Ok(results)
    }

    async fn get_our_validator_node(&self, epoch: Epoch) -> Result<ValidatorNode<Self::Addr>, EpochManagerError>;
    async fn get_local_committee_info(&self, epoch: Epoch) -> Result<CommitteeInfo, EpochManagerError>;
    async fn get_committee_info_for_substate(
        &self,
        epoch: Epoch,
        shard: SubstateAddress,
    ) -> Result<CommitteeInfo, EpochManagerError>;

    async fn get_committee_info_by_validator_public_key(
        &self,
        epoch: Epoch,
        public_key: &PublicKey,
    ) -> Result<CommitteeInfo, EpochManagerError> {
        let validator = self.get_validator_node_by_public_key(epoch, public_key).await?;
        self.get_committee_info_for_substate(epoch, validator.shard_key).await
    }

    async fn current_epoch(&self) -> Result<Epoch, EpochManagerError>;
    async fn current_base_layer_block_info(&self) -> Result<(u64, FixedHash), EpochManagerError>;
    async fn get_last_block_of_current_epoch(&self) -> Result<FixedHash, EpochManagerError>;
    async fn is_last_block_of_epoch(&self, block_height: u64) -> Result<bool, EpochManagerError>;
    async fn is_epoch_active(&self, epoch: Epoch) -> Result<bool, EpochManagerError>;

    async fn get_num_committees(&self, epoch: Epoch) -> Result<u32, EpochManagerError>;

    async fn get_committees_by_shards(
        &self,
        epoch: Epoch,
        shards: HashSet<Shard>,
    ) -> Result<HashMap<Shard, Committee<Self::Addr>>, EpochManagerError>;

    async fn get_local_committee(&self, epoch: Epoch) -> Result<Committee<Self::Addr>, EpochManagerError> {
        let validator = self.get_our_validator_node(epoch).await?;
        let committee = self.get_committee_for_substate(epoch, validator.shard_key).await?;
        Ok(committee)
    }

    async fn get_committee_by_validator_public_key(
        &self,
        epoch: Epoch,
        public_key: &PublicKey,
    ) -> Result<Committee<Self::Addr>, EpochManagerError> {
        let validator = self.get_validator_node_by_public_key(epoch, public_key).await?;
        let committee = self.get_committee_for_substate(epoch, validator.shard_key).await?;
        Ok(committee)
    }

    /// Returns true if the validator is in the local committee for the given epoch.
    /// It is recommended that implementations override this method if they can provide a more efficient implementation.
    async fn is_validator_in_local_committee(
        &self,
        validator_addr: &Self::Addr,
        epoch: Epoch,
    ) -> Result<bool, EpochManagerError> {
        let committee = self.get_local_committee(epoch).await?;
        Ok(committee.contains(validator_addr))
    }

    async fn get_current_epoch_committee(
        &self,
        shard: SubstateAddress,
    ) -> Result<Committee<Self::Addr>, EpochManagerError> {
        let current_epoch = self.current_epoch().await?;
        self.get_committee_for_substate(current_epoch, shard).await
    }

    async fn get_local_threshold_for_epoch(&self, epoch: Epoch) -> Result<usize, EpochManagerError> {
        let committee = self.get_local_committee_info(epoch).await?;
        Ok(committee.quorum_threshold() as usize)
    }

    async fn is_this_validator_registered_for_epoch(&self, epoch: Epoch) -> Result<bool, EpochManagerError> {
        if !self.is_epoch_active(epoch).await? {
            return Ok(false);
        }

        // TODO: might want to improve this
        self.get_local_committee_info(epoch).await.map(|_| true).or_else(|err| {
            if err.is_not_registered_error() {
                Ok(false)
            } else {
                Err(err)
            }
        })
    }

    async fn get_base_layer_block_height(&self, hash: FixedHash) -> Result<Option<u64>, EpochManagerError>;
}
