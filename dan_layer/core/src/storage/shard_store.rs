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

use std::collections::HashMap;

use tari_dan_common_types::{PayloadId, ShardId, SubstateChange, SubstateState};
use tari_engine_types::commit_result::FinalizeResult;
use thiserror::Error;

use crate::{
    models::{
        vote_message::VoteMessage,
        HotStuffTreeNode,
        LeafNode,
        NodeHeight,
        ObjectPledge,
        Payload,
        QuorumCertificate,
        RecentTransaction,
        SQLSubstate,
        SQLTransaction,
        SubstateShardData,
        TreeNodeHash,
    },
    services::infrastructure_services::NodeAddressable,
    storage::StorageError,
};

pub trait ShardStoreFactory {
    type Addr: NodeAddressable;
    type Payload: Payload;

    type Transaction: ShardStoreTransaction<Self::Addr, Self::Payload>;
    fn create_tx(&self) -> Result<Self::Transaction, StorageError>;
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
    fn count_high_qc_for(&self, shard_id: ShardId) -> Result<usize, StorageError>;
    fn update_high_qc(&mut self, from: TAddr, shard: ShardId, qc: QuorumCertificate) -> Result<(), StorageError>;
    fn set_payload(&mut self, payload: TPayload) -> Result<(), StorageError>;
    /// Returns the current leaf node for the shard, or the genesis node, height are returned.
    fn get_leaf_node(&self, shard: ShardId) -> Result<LeafNode, StorageError>;
    fn update_leaf_node(&mut self, shard: ShardId, node: TreeNodeHash, height: NodeHeight) -> Result<(), StorageError>;
    fn get_high_qc_for(&self, shard: ShardId) -> Result<QuorumCertificate, StorageError>;
    fn get_payload(&self, payload_id: &PayloadId) -> Result<TPayload, StorageError>;
    fn get_node(&self, node_hash: &TreeNodeHash) -> Result<HotStuffTreeNode<TAddr, TPayload>, StorageError>;
    fn save_node(&mut self, node: HotStuffTreeNode<TAddr, TPayload>) -> Result<(), StorageError>;
    fn get_locked_node_hash_and_height(&self, shard: ShardId) -> Result<(TreeNodeHash, NodeHeight), StorageError>;
    fn set_locked(
        &mut self,
        shard: ShardId,
        node_hash: TreeNodeHash,
        node_height: NodeHeight,
    ) -> Result<(), StorageError>;
    fn pledge_object(
        &mut self,
        shard: ShardId,
        payload: PayloadId,
        change: SubstateChange,
        current_height: NodeHeight,
    ) -> Result<ObjectPledge, StorageError>;
    fn set_last_executed_height(&mut self, shard: ShardId, height: NodeHeight) -> Result<(), StorageError>;
    fn get_last_executed_height(&self, shard: ShardId) -> Result<NodeHeight, StorageError>;
    fn save_substate_changes(
        &mut self,
        changes: &HashMap<ShardId, Vec<SubstateState>>,
        node: &HotStuffTreeNode<TAddr, TPayload>,
    ) -> Result<(), StorageError>;
    fn insert_substates(&mut self, substate_data: SubstateShardData) -> Result<(), StorageError>;
    fn get_state_inventory(&self) -> Result<Vec<ShardId>, StorageError>;
    fn get_substate_states(
        &self,
        start_shard_id: ShardId,
        end_shard_id: ShardId,
        excluded_shards: &[ShardId],
    ) -> Result<Vec<SubstateShardData>, StorageError>;
    fn get_last_voted_height(&self, shard: ShardId) -> Result<NodeHeight, StorageError>;
    fn set_last_voted_height(&mut self, shard: ShardId, height: NodeHeight) -> Result<(), StorageError>;
    fn get_leader_proposals(
        &self,
        payload: PayloadId,
        payload_height: NodeHeight,
        shard: ShardId,
    ) -> Result<Option<HotStuffTreeNode<TAddr, TPayload>>, StorageError>;
    fn save_leader_proposals(
        &mut self,
        shard: ShardId,
        payload: PayloadId,
        payload_height: NodeHeight,
        node: HotStuffTreeNode<TAddr, TPayload>,
    ) -> Result<(), StorageError>;
    fn has_vote_for(&self, from: &TAddr, node_hash: TreeNodeHash, shard: ShardId) -> Result<bool, StorageError>;
    fn save_received_vote_for(
        &mut self,
        from: TAddr,
        node_hash: TreeNodeHash,
        shard: ShardId,
        vote_message: VoteMessage,
    ) -> Result<usize, StorageError>;

    fn get_received_votes_for(&self, node_hash: TreeNodeHash, shard: ShardId)
        -> Result<Vec<VoteMessage>, StorageError>;
    fn get_recent_transactions(&self) -> Result<Vec<RecentTransaction>, StorageError>;
    fn get_transaction(&self, payload_id: Vec<u8>) -> Result<Vec<SQLTransaction>, StorageError>;
    fn get_substates(&self, payload_id: Vec<u8>, shard_id: Vec<u8>) -> Result<Vec<SQLSubstate>, StorageError>;
    fn update_payload_result(&self, payload_id: &PayloadId, result: FinalizeResult) -> Result<(), StorageError>;
}
