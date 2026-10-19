//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::collections::{HashMap, HashSet};

use tari_ootle_common_types::{Epoch, ShardGroup, SubstateAddress, VotePower, committee::Committee};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::global::{GlobalDbAdapter, models::ValidatorNode};

pub struct ValidatorNodeDb<'a, 'tx, TGlobalDbAdapter: GlobalDbAdapter> {
    backend: &'a TGlobalDbAdapter,
    tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>,
}

impl<'a, 'tx, TGlobalDbAdapter: GlobalDbAdapter> ValidatorNodeDb<'a, 'tx, TGlobalDbAdapter> {
    pub fn new(backend: &'a TGlobalDbAdapter, tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>) -> Self {
        Self { backend, tx }
    }

    pub fn insert_validator_node(
        &mut self,
        peer_address: TGlobalDbAdapter::Addr,
        public_key: RistrettoPublicKeyBytes,
        shard_key: SubstateAddress,
        start_epoch: Epoch,
        fee_claim_public_key: RistrettoPublicKeyBytes,
        power: VotePower,
    ) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend.insert_validator_node(
            self.tx,
            peer_address,
            public_key,
            shard_key,
            start_epoch,
            fee_claim_public_key,
            power,
        )
    }

    pub fn deactivate(
        &mut self,
        public_key: RistrettoPublicKeyBytes,
        deactivation_epoch: Epoch,
    ) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend
            .deactivate_validator_node(self.tx, public_key, deactivation_epoch)
    }

    pub fn count(&mut self, epoch: Epoch) -> Result<u64, TGlobalDbAdapter::Error> {
        self.backend.validator_nodes_count(self.tx, epoch)
    }

    pub fn count_in_shard_group(
        &mut self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<u64, TGlobalDbAdapter::Error> {
        self.backend
            .validator_nodes_count_for_shard_group(self.tx, epoch, shard_group)
    }

    pub fn get_by_public_key(
        &mut self,
        epoch: Epoch,
        public_key: &RistrettoPublicKeyBytes,
    ) -> Result<ValidatorNode<TGlobalDbAdapter::Addr>, TGlobalDbAdapter::Error> {
        self.backend
            .get_validator_node_by_public_key(self.tx, epoch, public_key)
    }

    pub fn get_by_address(
        &mut self,
        epoch: Epoch,
        address: &TGlobalDbAdapter::Addr,
    ) -> Result<ValidatorNode<TGlobalDbAdapter::Addr>, TGlobalDbAdapter::Error> {
        self.backend.get_validator_node_by_address(self.tx, epoch, address)
    }

    /// Returns all registered validator nodes from the given epoch
    ///
    /// This may be used to fetch validators registered for a future epoch, however since the epoch is not finalized
    /// yet, the list may not be complete.
    pub fn get_all_registered_within_start_epoch(
        &mut self,
        epoch: Epoch,
    ) -> Result<Vec<ValidatorNode<TGlobalDbAdapter::Addr>>, TGlobalDbAdapter::Error> {
        self.backend.get_validator_nodes_within_start_epoch(self.tx, epoch)
    }

    /// Fetches all validator nodes that are active for a given epoch
    pub fn get_all_within_epoch(
        &mut self,
        epoch: Epoch,
    ) -> Result<Vec<ValidatorNode<TGlobalDbAdapter::Addr>>, TGlobalDbAdapter::Error> {
        self.backend.get_validator_nodes_within_committee_epoch(self.tx, epoch)
    }

    pub fn get_committee_for_shard_group(
        &mut self,
        epoch: Epoch,
        shard_group: ShardGroup,
        limit: usize,
    ) -> Result<Committee<TGlobalDbAdapter::Addr>, TGlobalDbAdapter::Error> {
        self.backend
            .validator_nodes_get_for_shard_group(self.tx, epoch, shard_group, limit)
    }

    pub fn get_committees_overlapping_shard_group(
        &mut self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<HashMap<ShardGroup, Committee<TGlobalDbAdapter::Addr>>, TGlobalDbAdapter::Error> {
        self.backend
            .validator_nodes_get_overlapping_shard_group(self.tx, epoch, shard_group)
    }

    pub fn get_random_committee_member_from_shard_group(
        &mut self,
        epoch: Epoch,
        shard_group: Option<ShardGroup>,
        excluding: HashSet<TGlobalDbAdapter::Addr>,
    ) -> Result<ValidatorNode<TGlobalDbAdapter::Addr>, TGlobalDbAdapter::Error> {
        self.backend
            .validator_nodes_get_random_committee_member_from_shard_group(self.tx, epoch, shard_group, excluding)
    }

    /// Removes every committee assignment for `epoch`.
    pub fn clear_committees(&mut self, epoch: Epoch) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend.validator_nodes_clear_committees(self.tx, epoch)
    }

    pub fn set_committee_shard(
        &mut self,
        substate_address: SubstateAddress,
        shard_group: ShardGroup,
        epoch: Epoch,
    ) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend
            .validator_nodes_set_committee_shard(self.tx, substate_address, shard_group, epoch)
    }
}
