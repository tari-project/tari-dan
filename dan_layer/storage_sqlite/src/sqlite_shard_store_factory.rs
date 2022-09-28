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

use tari_common_types::types::PublicKey;
use tari_dan_common_types::{ObjectId, PayloadId, ShardId, SubstateChange, SubstateState};
use tari_dan_core::{
    models::{
        vote_message::VoteMessage,
        HotStuffTreeNode,
        NodeHeight,
        ObjectPledge,
        QuorumCertificate,
        TariDanPayload,
        TreeNodeHash,
    },
    storage::shard_store::{ShardStoreFactory, ShardStoreTransaction, StoreError},
};

pub struct SqliteShardStoreFactory {}

impl ShardStoreFactory for SqliteShardStoreFactory {
    type Addr = PublicKey;
    type Payload = TariDanPayload;
    type Transaction = SqliteShardStoreTransaction;

    fn create_tx(&self) -> Self::Transaction {
        SqliteShardStoreTransaction::new()
    }
}

pub struct SqliteShardStoreTransaction {}

impl SqliteShardStoreTransaction {
    fn new() -> Self {
        Self {}
    }
}

impl ShardStoreTransaction<PublicKey, TariDanPayload> for SqliteShardStoreTransaction {
    type Error = StoreError;

    fn commit(&mut self) -> Result<(), Self::Error> {
        todo!()
    }

    fn update_high_qc(&mut self, _shard: ShardId, _qc: QuorumCertificate) {
        todo!()
    }

    fn set_payload(&mut self, _payload: TariDanPayload) {
        todo!()
    }

    fn get_leaf_node(&self, _shard: ShardId) -> (TreeNodeHash, NodeHeight) {
        todo!()
    }

    fn update_leaf_node(
        &mut self,
        _shard: tari_dan_common_types::ShardId,
        _node: TreeNodeHash,
        _height: NodeHeight,
    ) -> Result<(), Self::Error> {
        todo!()
    }

    fn get_high_qc_for(&self, _shard: ShardId) -> QuorumCertificate {
        todo!()
    }

    fn get_payload(&self, _payload_id: &PayloadId) -> Result<TariDanPayload, Self::Error> {
        todo!()
    }

    fn get_node(&self, _node_hash: &TreeNodeHash) -> Result<HotStuffTreeNode<PublicKey>, Self::Error> {
        todo!()
    }

    fn save_node(&mut self, _node: HotStuffTreeNode<PublicKey>) {
        todo!()
    }

    fn get_locked_node_hash_and_height(&self, _shard: ShardId) -> (TreeNodeHash, NodeHeight) {
        todo!()
    }

    fn set_locked(&mut self, _shard: ShardId, _node_hash: TreeNodeHash, _node_height: NodeHeight) {
        todo!()
    }

    fn pledge_object(
        &mut self,
        _shard: ShardId,
        _object: ObjectId,
        _change: SubstateChange,
        _payload: PayloadId,
        _current_height: NodeHeight,
    ) -> ObjectPledge {
        todo!()
    }

    fn set_last_executed_height(&mut self, _shard: ShardId, _height: NodeHeight) {
        todo!()
    }

    fn get_last_executed_height(&self, _shard: ShardId) -> NodeHeight {
        todo!()
    }

    fn save_substate_changes(&mut self, _changes: HashMap<ShardId, Option<SubstateState>>, _node: TreeNodeHash) {
        todo!()
    }

    fn get_last_voted_height(&self, _shard: ShardId) -> NodeHeight {
        todo!()
    }

    fn set_last_voted_height(&mut self, _shard: ShardId, _height: NodeHeight) {
        todo!()
    }

    fn get_payload_vote(
        &self,
        _payload: PayloadId,
        _payload_height: NodeHeight,
        _shard: ShardId,
    ) -> Option<HotStuffTreeNode<PublicKey>> {
        todo!()
    }

    fn save_payload_vote(
        &mut self,
        _shard: ShardId,
        _payload: PayloadId,
        _payload_height: NodeHeight,
        _node: HotStuffTreeNode<PublicKey>,
    ) {
        todo!()
    }

    fn has_vote_for(&self, _from: &PublicKey, _node_hash: TreeNodeHash, _shard: ShardId) -> bool {
        todo!()
    }

    fn save_received_vote_for(
        &mut self,
        _from: PublicKey,
        _node_hash: TreeNodeHash,
        _shard: ShardId,
        _vote_message: VoteMessage,
    ) -> usize {
        todo!()
    }

    fn get_received_votes_for(&self, _node_hash: TreeNodeHash, _shard: ShardId) -> Vec<VoteMessage> {
        todo!()
    }
}
