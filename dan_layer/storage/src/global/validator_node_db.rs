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

use std::collections::HashMap;

use tari_common_types::types::PublicKey;
use tari_dan_common_types::{committee::Committee, Epoch, ShardGroup, SubstateAddress};

use crate::global::{models::ValidatorNode, GlobalDbAdapter};

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
        public_key: PublicKey,
        shard_key: SubstateAddress,
        registered_at_base_height: u64,
        start_epoch: Epoch,
        end_epoch: Epoch,
        fee_claim_public_key: PublicKey,
        sidechain_id: Option<PublicKey>,
    ) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend
            .insert_validator_node(
                self.tx,
                peer_address,
                public_key,
                shard_key,
                registered_at_base_height,
                start_epoch,
                end_epoch,
                fee_claim_public_key,
                sidechain_id,
            )
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn count(&mut self, epoch: Epoch, sidechain_id: Option<&PublicKey>) -> Result<u64, TGlobalDbAdapter::Error> {
        self.backend
            .validator_nodes_count(self.tx, epoch, sidechain_id)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn count_in_shard_group(
        &mut self,
        epoch: Epoch,
        sidechain_id: Option<&PublicKey>,
        shard_group: ShardGroup,
    ) -> Result<u64, TGlobalDbAdapter::Error> {
        self.backend
            .validator_nodes_count_for_shard_group(self.tx, epoch, sidechain_id, shard_group)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn get_by_public_key(
        &mut self,
        epoch: Epoch,
        public_key: &PublicKey,
        sidechain_id: Option<&PublicKey>,
    ) -> Result<ValidatorNode<TGlobalDbAdapter::Addr>, TGlobalDbAdapter::Error> {
        self.backend
            .get_validator_node_by_public_key(self.tx, epoch, public_key, sidechain_id)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn get_by_address(
        &mut self,
        epoch: Epoch,
        address: &TGlobalDbAdapter::Addr,
        sidechain_id: Option<&PublicKey>,
    ) -> Result<ValidatorNode<TGlobalDbAdapter::Addr>, TGlobalDbAdapter::Error> {
        self.backend
            .get_validator_node_by_address(self.tx, epoch, address, sidechain_id)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn get_all_within_epoch(
        &mut self,
        epoch: Epoch,
        sidechain_id: Option<&PublicKey>,
    ) -> Result<Vec<ValidatorNode<TGlobalDbAdapter::Addr>>, TGlobalDbAdapter::Error> {
        self.backend
            .get_validator_nodes_within_epoch(self.tx, epoch, sidechain_id)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn get_committees_for_shard_group(
        &mut self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<HashMap<ShardGroup, Committee<TGlobalDbAdapter::Addr>>, TGlobalDbAdapter::Error> {
        self.backend
            .validator_nodes_get_for_shard_group(self.tx, epoch, shard_group)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn get_committees(
        &mut self,
        epoch: Epoch,
        sidechain_id: Option<&PublicKey>,
    ) -> Result<HashMap<ShardGroup, Committee<TGlobalDbAdapter::Addr>>, TGlobalDbAdapter::Error> {
        self.backend
            .validator_nodes_get_committees_for_epoch(self.tx, epoch, sidechain_id)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn set_committee_shard(
        &mut self,
        substate_address: SubstateAddress,
        shard_group: ShardGroup,
        sidechain_id: Option<&PublicKey>,
        epoch: Epoch,
    ) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend
            .validator_nodes_set_committee_shard(self.tx, substate_address, shard_group, sidechain_id, epoch)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    /// Set new start/end epoch for the given validator nodes to let them available in the next epoch.
    pub fn increment_vn_start_end_epochs(
        &mut self,
        vn_addresses: Vec<String>,
    ) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend
            .increment_vn_start_end_epochs(self.tx, vn_addresses)
            .map_err(TGlobalDbAdapter::Error::into)
    }
}
