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

use std::ops::{Deref, DerefMut};

use tari_dan_common_types::{
    NodeAddressable,
    NodeHeight,
    ObjectPledge,
    ObjectPledgeInfo,
    PayloadId,
    QuorumCertificate,
    ShardId,
    SubstateState,
    TreeNodeHash,
};
use thiserror::Error;

use crate::{
    models::{
        vote_message::VoteMessage,
        HotStuffTreeNode,
        LeafNode,
        Payload,
        PayloadResult,
        RecentTransaction,
        SQLSubstate,
        SQLTransaction,
        SubstateShardData,
    },
    storage::StorageError,
};

const LOG_TARGET: &str = "tari::dan_layer::storage";

pub trait ShardStore {
    type Addr: NodeAddressable;
    type Payload: Payload;

    type ReadTransaction<'a>: ShardStoreReadTransaction<Self::Addr, Self::Payload>
    where Self: 'a;
    type WriteTransaction<'a>: ShardStoreWriteTransaction<Self::Addr, Self::Payload>
        + Deref<Target = Self::ReadTransaction<'a>>
        + DerefMut
    where Self: 'a;

    fn create_read_tx(&self) -> Result<Self::ReadTransaction<'_>, StorageError>;
    fn create_write_tx(&self) -> Result<Self::WriteTransaction<'_>, StorageError>;

    fn with_write_tx<F: FnOnce(&mut Self::WriteTransaction<'_>) -> Result<R, E>, R, E>(&self, f: F) -> Result<R, E>
    where E: From<StorageError> {
        let mut tx = self.create_write_tx()?;
        match f(&mut tx) {
            Ok(r) => {
                tx.commit()?;
                Ok(r)
            },
            Err(e) => {
                if let Err(err) = tx.rollback() {
                    log::error!(target: LOG_TARGET, "Failed to rollback transaction: {}", err);
                }
                Err(e)
            },
        }
    }

    fn with_read_tx<F: FnOnce(&mut Self::ReadTransaction<'_>) -> Result<R, E>, R, E>(&self, f: F) -> Result<R, E>
    where E: From<StorageError> {
        let mut tx = self.create_read_tx()?;
        let ret = f(&mut tx)?;
        Ok(ret)
    }
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("Cannot find payload")]
    CannotFindPayload,
    #[error("Cannot find node")]
    NodeNotFound,
    #[error("Cannot update leaf node")]
    CannotUpdateLeafNode,
    #[error("Storage error: {details}")]
    StorageError { details: String },
}

impl From<StorageError> for StoreError {
    fn from(err: StorageError) -> Self {
        Self::StorageError {
            details: err.to_string(),
        }
    }
}

pub trait ShardStoreReadTransaction<TAddr: NodeAddressable, TPayload: Payload> {
    fn get_high_qc_for(&mut self, payload_id: PayloadId, shard: ShardId) -> Result<QuorumCertificate, StorageError>;
    fn get_high_qcs(&mut self, payload_id: PayloadId) -> Result<Vec<QuorumCertificate>, StorageError>;
    /// Returns the current leaf node for the shard
    fn get_leaf_node(&mut self, payload_id: &PayloadId, shard: &ShardId) -> Result<LeafNode, StorageError>;
    fn get_payload(&mut self, payload_id: &PayloadId) -> Result<TPayload, StorageError>;
    fn get_node(&mut self, node_hash: &TreeNodeHash) -> Result<HotStuffTreeNode<TAddr, TPayload>, StorageError>;
    fn get_locked_node_hash_and_height(
        &mut self,
        payload_id: PayloadId,
        shard: ShardId,
    ) -> Result<(TreeNodeHash, NodeHeight), StorageError>;
    fn get_last_executed_height(&mut self, shard: ShardId, payload_id: PayloadId) -> Result<NodeHeight, StorageError>;
    fn get_state_inventory(&mut self) -> Result<Vec<ShardId>, StorageError>;
    fn get_substate_states(&mut self, shards: &[ShardId]) -> Result<Vec<SubstateShardData>, StorageError>;
    fn get_substate_states_by_range(
        &mut self,
        start_shard_id: ShardId,
        end_shard_id: ShardId,
        excluded_shards: &[ShardId],
    ) -> Result<Vec<SubstateShardData>, StorageError>;
    /// Returns the last voted height. A height of 0 means that no previous vote height has been recorded for the
    /// <shard, payload> pair.
    fn get_last_voted_height(
        &mut self,
        shard: ShardId,
        payload_id: PayloadId,
    ) -> Result<(NodeHeight, u32), StorageError>;
    fn get_leader_proposals(
        &mut self,
        payload: PayloadId,
        payload_height: NodeHeight,
        shards: &[ShardId],
    ) -> Result<Vec<HotStuffTreeNode<TAddr, TPayload>>, StorageError>;
    fn get_last_payload_height_for_leader_proposal(
        &mut self,
        payload: PayloadId,
        shard: ShardId,
    ) -> Result<NodeHeight, StorageError>;
    fn has_vote_for(&mut self, from: &TAddr, node_hash: TreeNodeHash) -> Result<bool, StorageError>;
    fn get_received_votes_for(&mut self, node_hash: TreeNodeHash) -> Result<Vec<VoteMessage>, StorageError>;
    fn get_recent_transactions(&mut self) -> Result<Vec<RecentTransaction>, StorageError>;
    fn get_transaction(&mut self, payload_id: Vec<u8>) -> Result<Vec<SQLTransaction>, StorageError>;
    fn get_substates_for_payload(
        &mut self,
        payload_id: Vec<u8>,
        shard_id: Vec<u8>,
    ) -> Result<Vec<SQLSubstate>, StorageError>;
    fn get_payload_result(&mut self, payload_id: &PayloadId) -> Result<PayloadResult, StorageError>;
    fn get_resolved_pledges_for_payload(&mut self, payload: PayloadId) -> Result<Vec<ObjectPledgeInfo>, StorageError>;
}

pub trait ShardStoreWriteTransaction<TAddr: NodeAddressable, TPayload: Payload> {
    fn commit(self) -> Result<(), StorageError>;
    fn rollback(self) -> Result<(), StorageError>;
    fn insert_high_qc(&mut self, from: TAddr, shard: ShardId, qc: QuorumCertificate) -> Result<(), StorageError>;
    fn save_payload(&mut self, payload: TPayload) -> Result<(), StorageError>;
    /// Inserts or updates the leaf node for the shard
    fn set_leaf_node(
        &mut self,
        payload_id: PayloadId,
        shard: ShardId,
        node: TreeNodeHash,
        payload_height: NodeHeight,
        height: NodeHeight,
    ) -> Result<(), StorageError>;
    fn save_node(&mut self, node: HotStuffTreeNode<TAddr, TPayload>) -> Result<(), StorageError>;
    fn set_locked(
        &mut self,
        payload_id: PayloadId,
        shard: ShardId,
        node_hash: TreeNodeHash,
        node_height: NodeHeight,
    ) -> Result<(), StorageError>;

    fn set_last_executed_height(
        &mut self,
        shard: ShardId,
        payload_id: PayloadId,
        height: NodeHeight,
    ) -> Result<(), StorageError>;
    fn save_substate_changes(
        &mut self,
        node: HotStuffTreeNode<TAddr, TPayload>,
        changes: &[SubstateState],
    ) -> Result<(), StorageError>;
    fn insert_substates(&mut self, substate_data: SubstateShardData) -> Result<(), StorageError>;
    fn set_last_voted_height(
        &mut self,
        shard: ShardId,
        payload_id: PayloadId,
        height: NodeHeight,
        leader_round: u32,
    ) -> Result<(), StorageError>;

    fn save_leader_proposals(
        &mut self,
        shard: ShardId,
        payload: PayloadId,
        payload_height: NodeHeight,
        leader_round: u32,
        node: HotStuffTreeNode<TAddr, TPayload>,
    ) -> Result<(), StorageError>;

    fn save_received_vote_for(
        &mut self,
        from: TAddr,
        node_hash: TreeNodeHash,
        vote_message: VoteMessage,
    ) -> Result<(), StorageError>;

    /// Updates the result for an existing payload
    fn update_payload_result(&mut self, payload_id: &PayloadId, result: PayloadResult) -> Result<(), StorageError>;

    // -------------------------------- Pledges -------------------------------- //
    fn pledge_object(
        &mut self,
        shard: ShardId,
        payload: PayloadId,
        current_height: NodeHeight,
    ) -> Result<ObjectPledge, StorageError>;
    fn complete_pledges(
        &mut self,
        shard: ShardId,
        payload_id: PayloadId,
        node_hash: &TreeNodeHash,
    ) -> Result<(), StorageError>;
    fn abandon_pledges(
        &mut self,
        shard: ShardId,
        payload_id: PayloadId,
        node_hash: &TreeNodeHash,
    ) -> Result<(), StorageError>;

    fn save_burnt_utxo(
        &mut self,
        substate: &tari_engine_types::substate::Substate,
        commitment_address: String,
        shard_id: ShardId,
    ) -> Result<(), StorageError>;
}
