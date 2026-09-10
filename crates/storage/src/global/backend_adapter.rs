//  Copyright 2021. The Tari Project
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

use serde::{Serialize, de::DeserializeOwned};
use tari_common_types::types::FixedHash;
use tari_ootle_common_types::{Epoch, NodeAddressable, ShardGroup, SubstateAddress, VotePower, committee::Committee};
use tari_template_lib_types::{Hash32, TemplateAddress, crypto::RistrettoPublicKeyBytes};

use super::{BlockHeaderModel, EpochData, TemplateStatus};
use crate::{
    atomic::AtomicDb,
    global::{
        models::ValidatorNode,
        template_db::{DbTemplate, DbTemplateUpdate},
    },
};

pub trait GlobalDbAdapter: AtomicDb + Send + Sync + Clone {
    type Addr: NodeAddressable;

    fn get_metadata<T: DeserializeOwned>(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        key: &[u8],
    ) -> Result<Option<T>, Self::Error>;
    fn set_metadata<T: Serialize>(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        key: &[u8],
        value: &T,
    ) -> Result<(), Self::Error>;

    fn template_exists(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        key: &TemplateAddress,
        status: Option<TemplateStatus>,
    ) -> Result<bool, Self::Error>;

    fn set_status(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        key: &TemplateAddress,
        status: TemplateStatus,
    ) -> Result<(), Self::Error>;

    fn get_template(&self, tx: &mut Self::DbTransaction<'_>, key: &[u8]) -> Result<Option<DbTemplate>, Self::Error>;
    fn get_templates(&self, tx: &mut Self::DbTransaction<'_>, limit: usize) -> Result<Vec<DbTemplate>, Self::Error>;
    fn get_templates_by_addresses<'a, I: IntoIterator<Item = &'a TemplateAddress>>(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        addresses: I,
    ) -> Result<Vec<DbTemplate>, Self::Error>;
    fn get_pending_templates(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        limit: usize,
    ) -> Result<Vec<DbTemplate>, Self::Error>;

    fn insert_template(&self, tx: &mut Self::DbTransaction<'_>, template: DbTemplate) -> Result<(), Self::Error>;
    fn update_template(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        key: &[u8],
        template: DbTemplateUpdate,
    ) -> Result<(), Self::Error>;

    fn insert_validator_node(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        address: Self::Addr,
        public_key: RistrettoPublicKeyBytes,
        shard_key: SubstateAddress,
        start_epoch: Epoch,
        fee_claim_public_key: RistrettoPublicKeyBytes,
        power: VotePower,
    ) -> Result<(), Self::Error>;

    fn deactivate_validator_node(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        public_key: RistrettoPublicKeyBytes,
        deactivation_epoch: Epoch,
    ) -> Result<(), Self::Error>;

    fn get_validator_nodes_within_start_epoch(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
    ) -> Result<Vec<ValidatorNode<Self::Addr>>, Self::Error>;

    fn get_validator_nodes_within_committee_epoch(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
    ) -> Result<Vec<ValidatorNode<Self::Addr>>, Self::Error>;

    fn get_validator_node_by_address(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
        address: &Self::Addr,
    ) -> Result<ValidatorNode<Self::Addr>, Self::Error>;

    fn get_validator_node_by_public_key(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
        public_key: &RistrettoPublicKeyBytes,
    ) -> Result<ValidatorNode<Self::Addr>, Self::Error>;
    fn validator_nodes_count(&self, tx: &mut Self::DbTransaction<'_>, epoch: Epoch) -> Result<u64, Self::Error>;
    fn validator_nodes_count_for_shard_group(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<u64, Self::Error>;

    fn validator_nodes_set_committee_shard(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        shard_key: SubstateAddress,
        shard_group: ShardGroup,
        epoch: Epoch,
    ) -> Result<(), Self::Error>;

    fn validator_nodes_get_for_shard_group(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
        shard_group: ShardGroup,
        limit: usize,
    ) -> Result<Committee<Self::Addr>, Self::Error>;

    fn validator_nodes_get_overlapping_shard_group(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<HashMap<ShardGroup, Committee<Self::Addr>>, Self::Error>;

    fn validator_nodes_get_random_committee_member_from_shard_group(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
        shard_group: Option<ShardGroup>,
        excluding: HashSet<Self::Addr>,
    ) -> Result<ValidatorNode<Self::Addr>, Self::Error>;

    fn validator_nodes_get_committees_for_epoch(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
    ) -> Result<HashMap<ShardGroup, Committee<Self::Addr>>, Self::Error>;

    /// Upserts the epoch hash for `epoch`. Overwrites an existing row so the epoch manager can apply
    /// an oracle hash correction for an epoch it has already persisted (only done while the epoch is
    /// unlocked; locked epochs are guarded in `EpochManagerService::activate_epoch`).
    fn insert_epoch(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
        epoch_hash: FixedHash,
    ) -> Result<(), Self::Error>;
    fn get_epoch(&self, tx: &mut Self::DbTransaction<'_>, epoch: Epoch) -> Result<Option<EpochData>, Self::Error>;

    fn insert_block_header(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        header: BlockHeaderModel,
    ) -> Result<(), Self::Error>;

    fn get_block_header_by_hash(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        max_epoch: Epoch,
        block_hash: &Hash32,
    ) -> Result<BlockHeaderModel, Self::Error>;
    fn get_first_block_header_by_epoch(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
    ) -> Result<BlockHeaderModel, Self::Error>;

    /// Deletes every stored block header with a height strictly greater than `height`. Used to discard
    /// base-layer headers orphaned by a reorg so the canonical headers can be re-scanned in their place.
    /// Returns the number of rows deleted.
    fn delete_block_headers_above(&self, tx: &mut Self::DbTransaction<'_>, height: u64) -> Result<usize, Self::Error>;
}
