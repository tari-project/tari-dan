//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::{consensus_models, StorageError};
use time::PrimitiveDateTime;

use crate::{
    serialization::{deserialize_hex_try_from, deserialize_json},
    sql_models,
};

#[derive(Debug, Clone, Queryable)]
pub struct Block {
    pub id: i32,
    pub block_id: String,
    pub parent_block_id: String,
    pub height: i64,
    pub epoch: i64,
    pub proposed_by: String,
    pub qc_id: String,
    pub commands: String,
    pub created_at: PrimitiveDateTime,
}

impl Block {
    pub fn try_convert(self, qc: sql_models::QuorumCertificate) -> Result<consensus_models::Block, StorageError> {
        Ok(consensus_models::Block::new(
            deserialize_hex_try_from(&self.parent_block_id)?,
            qc.try_into()?,
            NodeHeight(self.height as u64),
            Epoch(self.epoch as u64),
            deserialize_hex_try_from(&self.proposed_by)?,
            deserialize_json(&self.commands)?,
        ))
    }
}
