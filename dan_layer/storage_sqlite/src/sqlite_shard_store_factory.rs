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
    storage::shard_store::{ShardStoreFactory, ShardStoreTransaction},
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
    type Error = String;

    fn commit(&mut self) -> Result<(), Self::Error> {
        todo!()
    }

    fn update_high_qc(&mut self, shard: ShardId, qc: QuorumCertificate) {
        todo!()
    }

    fn set_payload(&mut self, payload: TariDanPayload) {
        todo!()
    }

    fn get_leaf_node(&self, shard: ShardId) -> (TreeNodeHash, NodeHeight) {
        todo!()
    }

    fn update_leaf_node(
        &mut self,
        shard: tari_dan_common_types::ShardId,
        node: TreeNodeHash,
        height: NodeHeight,
    ) -> Result<(), String> {
        todo!()
    }

    fn get_high_qc_for(&self, shard: ShardId) -> QuorumCertificate {
        todo!()
    }

    fn get_payload(&self, payload_id: &PayloadId) -> Option<TariDanPayload> {
        todo!()
    }

    fn get_node(&self, node_hash: &TreeNodeHash) -> Option<HotStuffTreeNode<PublicKey>> {
        todo!()
    }

    fn save_node(&mut self, node: HotStuffTreeNode<PublicKey>) {
        todo!()
    }

    fn get_locked_node_hash_and_height(&self, shard: ShardId) -> (TreeNodeHash, NodeHeight) {
        todo!()
    }

    fn set_locked(&mut self, shard: ShardId, node_hash: TreeNodeHash, node_height: NodeHeight) {
        todo!()
    }

    fn pledge_object(
        &mut self,
        shard: ShardId,
        object: ObjectId,
        change: SubstateChange,
        payload: PayloadId,
        current_height: NodeHeight,
    ) -> ObjectPledge {
        todo!()
    }

    fn set_last_executed_height(&mut self, shard: ShardId, height: NodeHeight) {
        todo!()
    }

    fn get_last_executed_height(&self, shard: ShardId) -> NodeHeight {
        todo!()
    }

    fn save_substate_changes(&mut self, changes: HashMap<ShardId, Option<SubstateState>>, node: TreeNodeHash) {
        todo!()
    }

    fn get_last_voted_height(&self, shard: ShardId) -> NodeHeight {
        todo!()
    }

    fn set_last_voted_height(&mut self, shard: ShardId, height: NodeHeight) {
        todo!()
    }

    fn get_payload_vote(
        &self,
        payload: PayloadId,
        payload_height: NodeHeight,
        shard: ShardId,
    ) -> Option<HotStuffTreeNode<PublicKey>> {
        todo!()
    }

    fn save_payload_vote(
        &mut self,
        shard: ShardId,
        payload: PayloadId,
        payload_height: NodeHeight,
        node: HotStuffTreeNode<PublicKey>,
    ) {
        todo!()
    }

    fn has_vote_for(&self, from: &PublicKey, node_hash: TreeNodeHash, shard: ShardId) -> bool {
        todo!()
    }

    fn save_received_vote_for(
        &mut self,
        from: PublicKey,
        node_hash: TreeNodeHash,
        shard: ShardId,
        vote_message: VoteMessage,
    ) -> usize {
        todo!()
    }

    fn get_received_votes_for(&self, node_hash: TreeNodeHash, shard: ShardId) -> Vec<VoteMessage> {
        todo!()
    }
}
