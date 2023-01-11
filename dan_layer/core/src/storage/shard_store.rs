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
use tari_engine_types::commit_result::FinalizeResult;
use thiserror::Error;

use crate::{
    models::{
        vote_message::VoteMessage,
        HotStuffTreeNode,
        LeafNode,
        Payload,
        RecentTransaction,
        SQLSubstate,
        SQLTransaction,
        SubstateShardData,
    },
    storage::StorageError,
};

pub trait ShardStore {
    type Addr: NodeAddressable;
    type Payload: Payload;

    // TODO: lmdb has read/write transactions so we'll need provide both in this trait
    type Transaction<'a>: ShardStoreTransaction<Self::Addr, Self::Payload>
    where Self: 'a;

    fn create_tx(&self) -> Result<Self::Transaction<'_>, StorageError>;

    fn with_write_tx<F: FnOnce(&mut Self::Transaction<'_>) -> Result<R, E>, R, E>(&self, f: F) -> Result<R, E>
    where E: From<StorageError> {
        let mut tx = self.create_tx()?;
        let ret = f(&mut tx)?;
        tx.commit()?;
        Ok(ret)
    }

    fn with_read_tx<F: FnOnce(&Self::Transaction<'_>) -> Result<R, E>, R, E>(&self, f: F) -> Result<R, E>
    where E: From<StorageError> {
        let tx = self.create_tx()?;
        let ret = f(&tx)?;
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

pub trait ShardStoreTransaction<TAddr: NodeAddressable, TPayload: Payload> {
    fn commit(self) -> Result<(), StorageError>;
    fn get_high_qc_for(&self, payload_id: PayloadId, shard: ShardId) -> Result<QuorumCertificate, StorageError>;
    fn insert_high_qc(&mut self, from: TAddr, shard: ShardId, qc: QuorumCertificate) -> Result<(), StorageError>;
    fn save_payload(&mut self, payload: TPayload) -> Result<(), StorageError>;
    /// Returns the current leaf node for the shard
    fn get_leaf_node(&self, payload_id: &PayloadId, shard: &ShardId) -> Result<LeafNode, StorageError>;
    /// Inserts or updates the leaf node for the shard
    fn set_leaf_node(
        &mut self,
        payload_id: PayloadId,
        shard: ShardId,
        node: TreeNodeHash,
        payload_height: NodeHeight,
        height: NodeHeight,
    ) -> Result<(), StorageError>;
    fn get_payload(&self, payload_id: &PayloadId) -> Result<TPayload, StorageError>;
    fn get_node(&self, node_hash: &TreeNodeHash) -> Result<HotStuffTreeNode<TAddr, TPayload>, StorageError>;
    fn save_node(&mut self, node: HotStuffTreeNode<TAddr, TPayload>) -> Result<(), StorageError>;
    fn get_locked_node_hash_and_height(
        &self,
        payload_id: PayloadId,
        shard: ShardId,
    ) -> Result<(TreeNodeHash, NodeHeight), StorageError>;
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
    fn get_last_executed_height(&self, shard: ShardId, payload_id: PayloadId) -> Result<NodeHeight, StorageError>;
    fn save_substate_changes(
        &mut self,
        node: HotStuffTreeNode<TAddr, TPayload>,
        changes: &[SubstateState],
    ) -> Result<(), StorageError>;
    fn insert_substates(&mut self, substate_data: SubstateShardData) -> Result<(), StorageError>;
    fn get_state_inventory(&self) -> Result<Vec<ShardId>, StorageError>;
    fn get_substate_states(&self, shards: &[ShardId]) -> Result<Vec<SubstateShardData>, StorageError>;
    fn get_substate_states_by_range(
        &self,
        start_shard_id: ShardId,
        end_shard_id: ShardId,
        excluded_shards: &[ShardId],
    ) -> Result<Vec<SubstateShardData>, StorageError>;
    /// Returns the last voted height. A height of 0 means that no previous vote height has been recorded for the
    /// <shard, payload> pair.
    fn get_last_voted_height(&self, shard: ShardId, payload_id: PayloadId) -> Result<NodeHeight, StorageError>;
    fn set_last_voted_height(
        &mut self,
        shard: ShardId,
        payload_id: PayloadId,
        height: NodeHeight,
    ) -> Result<(), StorageError>;
    fn get_leader_proposals(
        &self,
        payload: PayloadId,
        payload_height: NodeHeight,
        shards: &[ShardId],
    ) -> Result<Vec<HotStuffTreeNode<TAddr, TPayload>>, StorageError>;
    fn save_leader_proposals(
        &mut self,
        shard: ShardId,
        payload: PayloadId,
        payload_height: NodeHeight,
        node: HotStuffTreeNode<TAddr, TPayload>,
    ) -> Result<(), StorageError>;
    fn has_vote_for(&self, from: &TAddr, node_hash: TreeNodeHash) -> Result<bool, StorageError>;
    fn save_received_vote_for(
        &mut self,
        from: TAddr,
        node_hash: TreeNodeHash,
        vote_message: VoteMessage,
    ) -> Result<(), StorageError>;

    fn get_received_votes_for(&self, node_hash: TreeNodeHash) -> Result<Vec<VoteMessage>, StorageError>;
    fn get_recent_transactions(&self) -> Result<Vec<RecentTransaction>, StorageError>;
    fn get_transaction(&self, payload_id: Vec<u8>) -> Result<Vec<SQLTransaction>, StorageError>;
    fn get_substates_for_payload(
        &self,
        payload_id: Vec<u8>,
        shard_id: Vec<u8>,
    ) -> Result<Vec<SQLSubstate>, StorageError>;
    fn get_payload_result(&self, payload_id: &PayloadId) -> Result<FinalizeResult, StorageError>;
    /// Updates the result for an existing payload
    fn update_payload_result(&self, payload_id: &PayloadId, result: FinalizeResult) -> Result<(), StorageError>;

    // -------------------------------- Pledges -------------------------------- //
    fn pledge_object(
        &mut self,
        shard: ShardId,
        payload: PayloadId,
        current_height: NodeHeight,
    ) -> Result<ObjectPledge, StorageError>;
    fn get_resolved_pledges_for_payload(&self, payload: PayloadId) -> Result<Vec<ObjectPledgeInfo>, StorageError>;
    fn complete_pledges(
        &self,
        shard: ShardId,
        payload_id: PayloadId,
        node_hash: &TreeNodeHash,
    ) -> Result<(), StorageError>;
    fn abandon_pledges(
        &self,
        shard: ShardId,
        payload_id: PayloadId,
        node_hash: &TreeNodeHash,
    ) -> Result<(), StorageError>;
}
