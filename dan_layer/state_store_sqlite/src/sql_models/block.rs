//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::{Queryable, QueryableByName};
use serde::Serialize;
use tari_dan_common_types::{Epoch, NodeAddressable, NodeHeight};
use tari_dan_storage::{consensus_models, StorageError};
use time::PrimitiveDateTime;

use crate::{
    schema::blocks,
    serialization::{deserialize_hex, deserialize_hex_try_from, deserialize_json},
    sql_models,
};

#[derive(Debug, Clone, Queryable, QueryableByName)]
pub struct Block {
    pub id: i32,
    pub block_id: String,
    pub parent_block_id: String,
    pub height: i64,
    pub epoch: i64,
    pub proposed_by: String,
    pub qc_id: String,
    pub command_count: i64,
    pub commands: String,
    pub total_leader_fee: i64,
    pub is_committed: bool,
    pub is_processed: bool,
    pub is_dummy: bool,
    pub created_at: PrimitiveDateTime,
}

impl Block {
    pub fn try_convert<TAddr: NodeAddressable + Serialize>(
        self,
        qc: sql_models::QuorumCertificate,
    ) -> Result<consensus_models::Block<TAddr>, StorageError> {
        Ok(consensus_models::Block::load(
            deserialize_hex_try_from(&self.block_id)?,
            deserialize_hex_try_from(&self.parent_block_id)?,
            qc.try_into()?,
            NodeHeight(self.height as u64),
            Epoch(self.epoch as u64),
            TAddr::from_bytes(&deserialize_hex(&self.proposed_by)?).ok_or_else(|| StorageError::DecodingError {
                operation: "try_convert",
                item: "block",
                details: format!("Block #{} proposed_by is malformed", self.id),
            })?,
            deserialize_json(&self.commands)?,
            self.total_leader_fee as u64,
            self.is_dummy,
            self.is_processed,
            self.is_committed,
        ))
    }
}
