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

use std::{collections::HashMap, fmt::Display};

use tari_dan_common_types::{ObjectId, PayloadId, ShardId, SubstateChange, SubstateState};
use thiserror::Error;

use crate::{
    models::{
        vote_message::VoteMessage,
        HotStuffTreeNode,
        NodeHeight,
        ObjectPledge,
        Payload,
        QuorumCertificate,
        TreeNodeHash,
    },
    services::infrastructure_services::NodeAddressable,
    storage::shard_db::MemoryShardDb,
};

pub trait ShardStoreFactory {
    type Addr: NodeAddressable;
    type Payload: Payload;

    type Transaction: ShardStoreTransaction<Self::Addr, Self::Payload>;
    fn create_tx(&self) -> Self::Transaction;
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("Cannot find payload")]
    CannotFindPayload,
    #[error("Cannot find node")]
    NodeNotFound,
    #[error("Cannot update leaf node")]
    CannotUpdateLeafNode,
}

pub trait ShardStoreTransaction<TAddr: NodeAddressable, TPayload: Payload> {
    type Error: Display + Into<StoreError>;
    fn commit(&mut self) -> Result<(), Self::Error>;
    fn update_high_qc(&mut self, shard: ShardId, qc: QuorumCertificate);
    fn set_payload(&mut self, payload: TPayload);
    fn get_leaf_node(&self, shard: ShardId) -> (TreeNodeHash, NodeHeight);
    fn update_leaf_node(&mut self, shard: ShardId, node: TreeNodeHash, height: NodeHeight) -> Result<(), StoreError>;
    fn get_high_qc_for(&self, shard: ShardId) -> QuorumCertificate;
    fn get_payload(&self, payload_id: &PayloadId) -> Result<TPayload, Self::Error>;
    fn get_node(&self, node_hash: &TreeNodeHash) -> Result<HotStuffTreeNode<TAddr>, Self::Error>;
    fn save_node(&mut self, node: HotStuffTreeNode<TAddr>);
    fn get_locked_node_hash_and_height(&self, shard: ShardId) -> (TreeNodeHash, NodeHeight);
    fn set_locked(&mut self, shard: ShardId, node_hash: TreeNodeHash, node_height: NodeHeight);
    fn pledge_object(
        &mut self,
        shard: ShardId,
        object: ObjectId,
        change: SubstateChange,
        payload: PayloadId,
        current_height: NodeHeight,
    ) -> ObjectPledge;
    fn set_last_executed_height(&mut self, shard: ShardId, height: NodeHeight);
    fn get_last_executed_height(&self, shard: ShardId) -> NodeHeight;
    fn save_substate_changes(&mut self, changes: HashMap<ShardId, Option<SubstateState>>, node: TreeNodeHash);
    fn get_last_voted_height(&self, shard: ShardId) -> NodeHeight;
    fn set_last_voted_height(&mut self, shard: ShardId, height: NodeHeight);
    fn get_payload_vote(
        &self,
        payload: PayloadId,
        payload_height: NodeHeight,
        shard: ShardId,
    ) -> Option<HotStuffTreeNode<TAddr>>;
    fn save_payload_vote(
        &mut self,
        shard: ShardId,
        payload: PayloadId,
        payload_height: NodeHeight,
        node: HotStuffTreeNode<TAddr>,
    );
    fn has_vote_for(&self, from: &TAddr, node_hash: TreeNodeHash, shard: ShardId) -> bool;
    fn save_received_vote_for(
        &mut self,
        from: TAddr,
        node_hash: TreeNodeHash,
        shard: ShardId,
        vote_message: VoteMessage,
    ) -> usize;

    fn get_received_votes_for(&self, node_hash: TreeNodeHash, shard: ShardId) -> Vec<VoteMessage>;
}

pub struct MemoryShardStoreFactory<TAddr: NodeAddressable, TPayload: Payload> {
    inner: MemoryShardDb<TAddr, TPayload>,
}
impl<TAddr: NodeAddressable, TPayload: Payload> ShardStoreFactory for MemoryShardStoreFactory<TAddr, TPayload> {
    type Addr = TAddr;
    type Payload = TPayload;
    type Transaction = MemoryShardDb<TAddr, TPayload>;

    fn create_tx(&self) -> Self::Transaction {
        self.inner.clone()
    }
}
