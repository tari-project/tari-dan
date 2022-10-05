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

use borsh::{BorshDeserialize, BorshSerialize};
use diesel::{prelude::*, SqliteConnection};
use serde_json::json;
use tari_common_types::types::{FixedHash, PrivateKey, PublicKey, Signature};
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
    storage::{
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
        leaf_nodes::{LeafNode, NewLeafNode},
        payload::{Payload, NewPayload},
    },
    schema::{
        high_qcs::{dsl::high_qcs, shard_id},
        leaf_nodes::dsl::leaf_nodes,
        payloads::dsl::payloads,
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
            let instructions = Vec::<Instruction>::deserialize(&mut payload.instructions.as_slice())
                .expect("TODO: Need to return an error");
            let fee: u64 = payload.fee.try_into().expect("TODO: Need to return an error");

            let public_nonce = PublicKey::from_vec(&payload.public_nonce).expect("TODO: Need to return an error");
            let signature = PrivateKey::from_bytes(payload.scalar.as_slice()).expect("TODO: Need to return an error");

            let signature = InstructionSignature::try_from(Signature::new(public_nonce, signature))
                .expect("TODO: Need to return an error");

            let sender_public_key =
                PublicKey::from_vec(&payload.sender_public_key).expect("TODO: Need to return an error");
            let meta =
                TransactionMeta::deserialize(&mut payload.meta.as_slice()).expect("TODO: Need to return an error");

            let transaction = Transaction::new(fee, instructions, signature, sender_public_key, meta);

            Ok(TariDanPayload::new(transaction))
        } else {
            panic!("TODO: Need to return an error");
        }
    }

    fn set_payload(&mut self, payload: TariDanPayload) {
        let transaction = payload.transaction();

        let mut instructions = Vec::<u8>::new();
        transaction.instructions().serialize(&mut instructions).expect("TODO: Need to return an error");
        let signature = transaction.signature();

        let public_nonce = Vec::from(signature.signature().get_public_nonce().as_bytes());
        let scalar = Vec::from(signature.signature().get_signature().as_bytes());

        let fee: i32 = transaction.fee().try_into().expect("TODO: Need to return an error");
        let sender_public_key = Vec::from(transaction.sender_public_key().as_bytes());
        let mut meta = Vec::<u8>::new();
        transaction.meta().serialize(&mut meta).expect("TODO: Need to return an error");

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
