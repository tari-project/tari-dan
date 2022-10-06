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

use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
    path::PathBuf,
};

use diesel::{prelude::*, SqliteConnection, dsl::max};
use serde_json::json;
use tari_common_types::types::{PrivateKey, PublicKey, Signature};
use tari_dan_common_types::{ObjectId, PayloadId, ShardId, SubstateChange, SubstateState};
use tari_dan_core::{
    models::{
        vote_message::VoteMessage,
        Epoch,
        HotStuffTreeNode,
        NodeHeight,
        ObjectPledge,
        QuorumCertificate,
        TariDanPayload,
        TreeNodeHash,
    },
    storage::{
        deserialize,
        serialize,
        shard_store::{ShardStoreFactory, ShardStoreTransaction},
        StorageError,
    },
};
use tari_dan_engine::instruction::{Instruction, InstructionSignature, Transaction, TransactionMeta};
use tari_utilities::ByteArray;

use crate::{
    error::SqliteStorageError,
    models::{
        high_qc::{HighQc, NewHighQc},
        last_executed_height::{LastExecutedHeight, NewLastExecutedHeight},
        last_voted_height::{LastVotedHeight, NewLastVotedHeight},
        leaf_nodes::{LeafNode, NewLeafNode},
        lock_node_and_height::{LockNodeAndHeight, NewLockNodeAndHeight},
        nodes::{NewNode, Node},
        payload::{NewPayload, Payload},
        payload_votes::{NewPayloadVote, PayloadVote},
        votes::{NewVote, Vote},
    },
    schema::{
        high_qcs::{dsl::high_qcs, shard_id},
        last_executed_heights::dsl::last_executed_heights,
        last_voted_heights::dsl::last_voted_heights,
        leaf_nodes::dsl::leaf_nodes,
        lock_node_and_heights::dsl::lock_node_and_heights,
        nodes::dsl::nodes as table_nodes,
        payload_votes::dsl::payload_votes,
        payloads::dsl::payloads,
        votes::dsl::votes,
    },
};

pub struct SqliteShardStoreFactory {
    url: PathBuf,
}

impl SqliteShardStoreFactory {
    pub fn try_create(url: PathBuf) -> Result<Self, StorageError> {
        let connection = SqliteConnection::establish(&url.clone().into_os_string().into_string().unwrap())
            .map_err(|e| StorageError::ConnectionError { reason: e.to_string() })?;

        embed_migrations!("./migrations");
        embedded_migrations::run(&connection).map_err(|e| StorageError::ConnectionError { reason: e.to_string() })?;
        Ok(Self { url })
    }
}
impl ShardStoreFactory for SqliteShardStoreFactory {
    type Addr = PublicKey;
    type Payload = TariDanPayload;
    type Transaction = SqliteShardStoreTransaction;

    fn create_tx(&self) -> Result<Self::Transaction, StorageError> {
        match SqliteConnection::establish(&self.url.clone().into_os_string().into_string().unwrap()) {
            Ok(connection) => {
                connection
                    .execute("PRAGMA foreign_keys = ON;   BEGIN TRANSACTION;")
                    .map_err(|source| SqliteStorageError::DieselError {
                        source,
                        operation: "set pragma".to_string(),
                    })?;
                Ok(SqliteShardStoreTransaction::new(connection))
            },
            Err(err) => Err(SqliteStorageError::from(err).into()),
        }
    }
}

pub struct SqliteShardStoreTransaction {
    connection: SqliteConnection,
}

impl SqliteShardStoreTransaction {
    fn new(connection: SqliteConnection) -> Self {
        Self { connection }
    }
}

impl ShardStoreTransaction<PublicKey, TariDanPayload> for SqliteShardStoreTransaction {
    type Error = StorageError;

    fn commit(&mut self) -> Result<(), Self::Error> {
        self.connection
            .execute("COMMIT TRANSACTION;")
            .map_err(|source| StorageError::QueryError {
                reason: format!("Commit transaction error: {0}", source),
            })?;
        Ok(())
    }

    fn update_high_qc(&mut self, shard: ShardId, qc: QuorumCertificate) -> Result<(), Self::Error> {
        // update all others for this shard to highest == false
        use crate::schema::high_qcs::{height, is_highest};
        let shard = Vec::from(shard.0);
        let num_existing_qcs: i64 = high_qcs
            .filter(shard_id.eq(&shard))
            .count()
            .first(&self.connection)
            .map_err(|source| StorageError::QueryError {
                reason: format!("Update qc error: {0}", source),
            })?;

        // TODO: fix i32 cast
        let rows = diesel::update(
            high_qcs.filter(
                shard_id
                    .eq(&shard)
                    .and(is_highest.eq(1))
                    .and(height.lt(qc.local_node_height().0 as i32)),
            ),
        )
        .set(is_highest.eq(0))
        .execute(&self.connection)
        .map_err(|e| StorageError::QueryError { reason: e.to_string() })?;

        let new_row = NewHighQc {
            shard_id: shard,
            height: qc.local_node_height().0 as i32,
            is_highest: if rows == 0 && num_existing_qcs > 0 { 0 } else { 1 },
            qc_json: json!(qc).to_string(),
        };
        diesel::insert_into(high_qcs)
            .values(&new_row)
            .execute(&self.connection)
            .map_err(|e| StorageError::QueryError { reason: e.to_string() })?;

        Ok(())
    }

    fn get_leaf_node(&self, shard: ShardId) -> (TreeNodeHash, NodeHeight) {
        use crate::schema::leaf_nodes::{node_height, shard_id};
        let leaf_node: Option<LeafNode> = leaf_nodes
            .filter(shard_id.eq(Vec::from(shard.0)))
            .order_by(node_height.desc())
            .first(&self.connection)
            .optional()
            .expect("TODO: Need to return an error");
        if let Some(leaf_node) = leaf_node {
            (
                TreeNodeHash::try_from(leaf_node.tree_node_hash).expect("TODO: Need to return an error"),
                NodeHeight(leaf_node.node_height as u64),
            )
        } else {
            panic!("TODO: Need to return an error");
        }
    }

    fn update_leaf_node(
        &mut self,
        shard: tari_dan_common_types::ShardId,
        node: TreeNodeHash,
        height: NodeHeight,
    ) -> Result<(), Self::Error> {
        let shard = Vec::from(shard.0);
        let tree_node_hash = Vec::from(node.as_bytes());

        let new_row = NewLeafNode {
            shard_id: shard,
            tree_node_hash,
            node_height: height.0 as i32,
        };

        diesel::insert_into(leaf_nodes)
            .values(&new_row)
            .execute(&self.connection)
            .map_err(|e| StorageError::QueryError { reason: e.to_string() })?;

        Ok(())
    }

    fn get_high_qc_for(&self, shard: ShardId) -> QuorumCertificate {
        use crate::schema::high_qcs::height;
        let qc: Option<HighQc> = high_qcs
            .filter(shard_id.eq(Vec::from(shard.0)))
            .order_by(height.desc())
            .first(&self.connection)
            .optional()
            .expect("Need to return an error");
        if let Some(qc) = qc {
            serde_json::from_str(&qc.qc_json).unwrap()
        } else {
            QuorumCertificate::genesis()
        }
    }

    fn get_payload(&self, id: &PayloadId) -> Result<TariDanPayload, Self::Error> {
        use crate::schema::payloads::payload_id;

        let payload: Option<Payload> = payloads
            .filter(payload_id.eq(Vec::from(id.as_slice())))
            .first(&self.connection)
            .optional()
            .expect("TODO: Need to return an error");

        if let Some(payload) = payload {
            let instructions =
                deserialize::<Vec<Instruction>>(&payload.instructions).expect("TODO: Need to return an error");

            let fee: u64 = payload.fee.try_into().expect("TODO: Need to return an error");

            let public_nonce = PublicKey::from_vec(&payload.public_nonce).expect("TODO: Need to return an error");
            let signature = PrivateKey::from_bytes(payload.scalar.as_slice()).expect("TODO: Need to return an error");

            let signature = InstructionSignature::try_from(Signature::new(public_nonce, signature))
                .expect("TODO: Need to return an error");

            let sender_public_key =
                PublicKey::from_vec(&payload.sender_public_key).expect("TODO: Need to return an error");
            let meta = deserialize::<TransactionMeta>(&payload.meta).expect("TODO: Need to return an error");

            let transaction = Transaction::new(fee, instructions, signature, sender_public_key, meta);

            Ok(TariDanPayload::new(transaction))
        } else {
            panic!("TODO: Need to return an error");
        }
    }

    fn set_payload(&mut self, payload: TariDanPayload) {
        let transaction = payload.transaction();
        let instructions = serialize(&transaction.instructions()).expect("TODO: Need to return an error");

        let signature = transaction.signature();

        let public_nonce = Vec::from(signature.signature().get_public_nonce().as_bytes());
        let scalar = Vec::from(signature.signature().get_signature().as_bytes());

        let fee: i32 = transaction.fee().try_into().expect("TODO: Need to return an error");
        let sender_public_key = Vec::from(transaction.sender_public_key().as_bytes());

        let meta = serialize(transaction.meta()).expect("TODO: Need to return an error");

        let payload_id = Vec::from(*transaction.hash());

        let new_row = NewPayload {
            payload_id,
            instructions,
            public_nonce,
            scalar,
            fee,
            sender_public_key,
            meta,
        };

        diesel::insert_into(payloads)
            .values(&new_row)
            .execute(&self.connection)
            .map_err(|e| StorageError::QueryError { reason: e.to_string() })
            .unwrap();
    }

    fn get_node(&self, node_hash: &TreeNodeHash) -> Result<HotStuffTreeNode<PublicKey>, Self::Error> {
        use crate::schema::nodes::{height, payload_height, tree_node_hash};
        let node_hash = Vec::from(node_hash.as_bytes());
        let node: Option<Node> = table_nodes
            .filter(tree_node_hash.eq(node_hash))
            .order_by(height.desc())
            .order_by(payload_height.desc())
            .first(&self.connection)
            .optional()
            .expect("TODO: Need to return an error");

        if let Some(node) = node {
            let mut parent = [0u8; 32];
            parent.copy_from_slice(node.parent_node_hash.as_slice());
            let parent = TreeNodeHash::from(parent);

            let hgt: u64 = node.height.try_into().expect("TODO: Need to return an error");

            let shard = deserialize::<ShardId>(&node.shard).expect("TODO: Need to return an error");
            let payload = deserialize::<PayloadId>(&node.payload_id.as_slice()).expect("TODO: Need to return an error");

            let payload_hgt: u64 = node.payload_height.try_into().expect("TODO: Need to return an error");
            let local_pledges = deserialize::<Vec<ObjectPledge>>(&node.local_pledges.as_slice())
                .expect("TODO: Need to return an error");

            let epoch: u64 = node.epoch.try_into().expect("TODO: Need to return an error");
            let proposed_by = PublicKey::from_vec(&node.proposed_by).expect("TODO: Need to return an error");

            let justify = deserialize::<QuorumCertificate>(&node.justify).expect("TODO: Need to return an error");

            Ok(HotStuffTreeNode::new(
                parent,
                shard,
                NodeHeight(hgt),
                payload,
                NodeHeight(payload_hgt),
                local_pledges,
                Epoch(epoch),
                proposed_by,
                justify,
            ))
        } else {
            panic!("TODO")
        }
    }

    fn save_node(&mut self, node: HotStuffTreeNode<PublicKey>) {
        let tree_node_hash = Vec::from(node.hash().as_bytes());
        let parent_node_hash = Vec::from(node.parent().as_bytes());

        let height = node.height().0.try_into().expect("TODO: Need to return an error");
        let shard = serialize(&node.shard()).expect("TODO: Need to return an error");

        let payload_id = serialize(&node.payload()).expect("TODO: Need to return an error");
        let payload_height: i32 = node
            .payload_height()
            .0
            .try_into()
            .expect("TODO: Need to return an error");

        let local_pledges = serialize(&node.local_pledges()).expect("TODO: Need to return an error");

        let epoch: i32 = node.epoch().0.try_into().expect("TODO: Need to return an error");
        let proposed_by = Vec::from(node.proposed_by().as_bytes());

        let justify = serialize(node.justify()).expect("TODO: Need to return an error");

        let new_row = NewNode {
            tree_node_hash,
            parent_node_hash,
            height,
            shard,
            payload_id,
            payload_height,
            local_pledges,
            epoch,
            proposed_by,
            justify,
        };

        diesel::insert_into(table_nodes)
            .values(&new_row)
            .execute(&self.connection)
            .map_err(|e| StorageError::QueryError { reason: e.to_string() })
            .expect("TODO: Need to return an error");
    }

    fn get_locked_node_hash_and_height(&self, shard: ShardId) -> (TreeNodeHash, NodeHeight) {
        use crate::schema::lock_node_and_heights::{node_height, shard_id};

        let shard = Vec::from(shard.0);

        let lock_node_hash_and_height: Option<LockNodeAndHeight> = lock_node_and_heights
            .filter(shard_id.eq(shard))
            .order_by(node_height.desc())
            .first(&self.connection)
            .optional()
            .expect("TODO: Need to return an error");

        if let Some(data) = lock_node_hash_and_height {
            let mut tree_node_hash = [0u8; 32];
            tree_node_hash.copy_from_slice(data.tree_node_hash.as_slice());
            let tree_node_hash = TreeNodeHash::from(tree_node_hash);

            let height: u64 = data.node_height.try_into().expect("TODO: Need to return an error");

            (tree_node_hash, NodeHeight(height))
        } else {
            panic!("TODO")
        }
    }

    fn set_locked(&mut self, shard: ShardId, node_hash: TreeNodeHash, node_height: NodeHeight) {
        let shard = Vec::from(shard.as_bytes());
        let node_hash = Vec::from(node_hash.as_bytes());
        let node_height = i32::try_from(node_height.0).expect("TODO: Return an error");

        let new_row = NewLockNodeAndHeight {
            shard_id: shard,
            tree_node_hash: node_hash,
            node_height,
        };

        diesel::insert_into(lock_node_and_heights)
            .values(&new_row)
            .execute(&self.connection)
            .map_err(|e| StorageError::QueryError { reason: e.to_string() })
            .expect("TODO: Need to return an error");
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

    fn set_last_executed_height(&mut self, shard: ShardId, height: NodeHeight) {
        let shard = Vec::from(shard.as_bytes());
        let node_height: i32 = height.0.try_into().expect("TODO: Need to return an error");

        let new_row = NewLastExecutedHeight {
            shard_id: shard,
            node_height,
        };

        diesel::insert_into(last_executed_heights)
            .values(&new_row)
            .execute(&self.connection)
            .map_err(|e| StorageError::QueryError { reason: e.to_string() })
            .expect("TODO: Need to return an error");
    }

    fn get_last_executed_height(&self, shard: ShardId) -> NodeHeight {
        use crate::schema::last_executed_heights::{node_height, shard_id};

        let last_executed_height: Option<LastExecutedHeight> = last_executed_heights
            .filter(shard_id.eq(Vec::from(shard.0)))
            .order_by(node_height.desc())
            .first(&self.connection)
            .optional()
            .expect("TODO: Need to return an error");

        if let Some(last_exec_height) = last_executed_height {
            let height = last_exec_height.node_height.try_into().expect("TODO: Return an error");
            NodeHeight(height)
        } else {
            panic!("TODO")
        }
    }

    fn save_substate_changes(&mut self, _changes: HashMap<ShardId, Option<SubstateState>>, _node: TreeNodeHash) {
        todo!()
    }

    fn get_last_voted_height(&self, shard: ShardId) -> NodeHeight {
        use crate::schema::last_voted_heights::{node_height, shard_id};

        let last_vote: Option<LastVotedHeight> = last_voted_heights
            .filter(shard_id.eq(Vec::from(shard.as_bytes())))
            .order_by(node_height.desc())
            .first(&self.connection)
            .optional()
            .expect("TODO: Need to return an error");

        if let Some(last_vote_height) = last_vote {
            let height = last_vote_height
                .node_height
                .try_into()
                .expect("TODO: Need to return an error");
            NodeHeight(height)
        } else {
            panic!("TODO")
        }
    }

    fn set_last_voted_height(&mut self, shard: ShardId, height: NodeHeight) {
        let shard = Vec::from(shard.as_bytes());
        let height: i32 = height.0.try_into().expect("TODO: Need to return an error");

        let new_row = NewLastVotedHeight {
            shard_id: shard,
            node_height: height,
        };

        diesel::insert_into(last_voted_heights)
            .values(&new_row)
            .execute(&self.connection)
            .map_err(|e| StorageError::QueryError { reason: e.to_string() })
            .expect("TODO: Need to return an error");
    }

    fn get_payload_vote(
        &self,
        payload: PayloadId,
        payload_height: NodeHeight,
        shard: ShardId,
    ) -> Option<HotStuffTreeNode<PublicKey>> {
        use crate::schema::payload_votes::{node_height, payload_id, shard_id};

        let payload_vote: Option<PayloadVote> = payload_votes
            .filter(
                payload_id
                    .eq(Vec::from(payload.as_slice()))
                    .and(shard_id.eq(Vec::from(shard.as_bytes())))
                    .and(node_height.eq(payload_height.0 as i32)),
            )
            .first(&self.connection)
            .optional()
            .expect("TODO: Need to return an error");

        if let Some(payload_vote) = payload_vote {
            let hot_stuff_tree_node = deserialize::<HotStuffTreeNode<PublicKey>>(&payload_vote.hotstuff_tree_node)
                .expect("TODO: Need to return an error");
            Some(hot_stuff_tree_node)
        } else {
            None
        }
    }

    fn save_payload_vote(
        &mut self,
        shard: ShardId,
        payload: PayloadId,
        payload_height: NodeHeight,
        node: HotStuffTreeNode<PublicKey>,
    ) {
        let shard = Vec::from(shard.as_bytes());
        let payload = Vec::from(payload.as_slice());
        let payload_height: i32 = payload_height.0.try_into().expect("TODO: Need to return an error");
        let node = serialize(&node).expect("TODO: Return an error");

        let new_row = NewPayloadVote {
            shard_id: shard,
            payload_id: payload,
            node_height: payload_height,
            hotstuff_tree_node: node,
        };

        diesel::insert_into(payload_votes)
            .values(&new_row)
            .execute(&self.connection)
            .map_err(|e| StorageError::QueryError { reason: e.to_string() })
            .expect("TODO: Need to return an error");
    }

    fn has_vote_for(&self, from: &PublicKey, node_hash: TreeNodeHash, shard: ShardId) -> bool {
        use crate::schema::votes::{address, node_height, shard_id, tree_node_hash};

        let vote: Option<Vote> = votes
            .filter(
                shard_id
                    .eq(Vec::from(shard.as_bytes()))
                    .and(tree_node_hash.eq(Vec::from(node_hash.as_bytes())))
                    .and(address.eq(Vec::from(from.as_bytes()))),
            )
            .order_by(node_height.desc())
            .first(&self.connection)
            .optional()
            .expect("TODO: Need to return an error");

        if let Some(_) = vote {
            true
        } else {
            false
        }
    }

    fn save_received_vote_for(
        &mut self,
        from: PublicKey,
        node_hash: TreeNodeHash,
        shard: ShardId,
        vote_message: VoteMessage,
    ) -> usize {
        use crate::schema::votes::{node_height, shard_id, tree_node_hash};

        let from = Vec::from(from.as_bytes());
        let node_hash = Vec::from(node_hash.as_bytes());
        let shard = Vec::from(shard.as_bytes());
        let vote_message = serialize(&vote_message).expect("TODO: Need to return an error");

        let current_node_height: Option<i32> = votes
            .select(max(node_height))
            .first(&self.connection)
            .map_err(|e| StorageError::QueryError { reason: e.to_string() })
            .expect("TODO: Need to return an error");

        let new_row = NewVote {
            tree_node_hash: node_hash.clone(),
            shard_id: shard.clone(),
            address: from,
            node_height: current_node_height.unwrap() + 1, // TODO: does every new received vote account for a higher height ? 
            vote_message,
        };

        diesel::insert_into(votes)
            .values(&new_row)
            .execute(&self.connection)
            .map_err(|e| StorageError::QueryError { reason: e.to_string() })
            .expect("TODO: Need to return an error");

        let count: i64 = votes
            .filter(
                tree_node_hash
                    .eq(Vec::from(node_hash.as_bytes()))
                    .and(shard_id.eq(Vec::from(shard.as_slice())))
                )
            .count()
            .first(&self.connection)
            .map_err(|source| StorageError::QueryError {
                reason: format!("Save received vote: {0}", source),
            })
            .expect("TODO: Need to return an error");
            
        count.try_into().expect("TODO: Need to return an error")
    }

    fn get_received_votes_for(&self, node_hash: TreeNodeHash, shard: ShardId) -> Vec<VoteMessage> {
        use crate::schema::votes::{tree_node_hash, shard_id, address,};

        let filtered_votes: Option<Vec<Vote>> = votes
            .filter(
                shard_id
                    .eq(Vec::from(shard.as_bytes()))
                    .and(tree_node_hash.eq(Vec::from(node_hash.as_bytes())))
            )
            .get_results(&self.connection)
            .optional()
            .expect("TODO: Need to return an error");
        
        if let Some(filtered_votes) = filtered_votes {
            filtered_votes
                .iter()
                .map(|v| deserialize::<VoteMessage>(&v.vote_message).expect("TODO: Need to return an error"))
                .collect::<Vec<_>>()
        } else {
            vec![]
        }
    }
}
