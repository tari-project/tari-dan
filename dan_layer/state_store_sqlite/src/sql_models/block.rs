//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::{Queryable, QueryableByName};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{Epoch, NodeHeight, ShardGroup};
use tari_dan_storage::{consensus_models, StorageError};
use tari_utilities::byte_array::ByteArray;
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
    pub merkle_root: String,
    pub network: String,
    pub height: i64,
    pub epoch: i64,
    pub shard_group: i32,
    pub proposed_by: String,
    pub qc_id: String,
    pub command_count: i64,
    pub commands: String,
    pub total_leader_fee: i64,
    pub is_committed: bool,
    pub is_justified: bool,
    pub is_dummy: bool,
    pub foreign_indexes: String,
    pub signature: Option<String>,
    pub block_time: Option<i64>,
    pub timestamp: i64,
    pub base_layer_block_height: i64,
    pub base_layer_block_hash: String,
    pub created_at: PrimitiveDateTime,
    pub extra_data: Option<String>,
}

impl Block {
    pub fn try_convert(self, qc: sql_models::QuorumCertificate) -> Result<consensus_models::Block, StorageError> {
        let network = self.network.parse().map_err(|_| StorageError::DecodingError {
            operation: "try_convert",
            item: "block",
            details: format!("Block #{} network byte is not a valid Network", self.id),
        })?;
        Ok(consensus_models::Block::load(
            deserialize_hex_try_from(&self.block_id)?,
            network,
            deserialize_hex_try_from(&self.parent_block_id)?,
            qc.try_into()?,
            NodeHeight(self.height as u64),
            Epoch(self.epoch as u64),
            ShardGroup::decode_from_u32(self.shard_group as u32).ok_or_else(|| StorageError::DataInconsistency {
                details: format!(
                    "Block id={} shard_group ({}) is not a valid",
                    self.id, self.shard_group as u32
                ),
            })?,
            PublicKey::from_canonical_bytes(&deserialize_hex(&self.proposed_by)?).map_err(|_| {
                StorageError::DecodingError {
                    operation: "try_convert",
                    item: "block",
                    details: format!("Block #{} proposed_by is malformed", self.id),
                }
            })?,
            deserialize_json(&self.commands)?,
            deserialize_hex_try_from(&self.merkle_root)?,
            self.total_leader_fee as u64,
            self.is_dummy,
            self.is_justified,
            self.is_committed,
            deserialize_json(&self.foreign_indexes)?,
            self.signature.map(|val| deserialize_json(&val)).transpose()?,
            self.created_at,
            self.block_time.map(|v| v as u64),
            self.timestamp as u64,
            self.base_layer_block_height as u64,
            deserialize_hex_try_from(&self.base_layer_block_hash)?,
            self.extra_data.map(|val| deserialize_json(&val)).transpose()?,
        ))
    }
}

#[derive(Debug, Clone, Queryable)]
pub struct ParkedBlock {
    pub id: i32,
    pub block_id: String,
    pub parent_block_id: String,
    pub merkle_root: String,
    pub network: String,
    pub height: i64,
    pub epoch: i64,
    pub shard_group: i32,
    pub proposed_by: String,
    pub justify: String,
    pub command_count: i64,
    pub commands: String,
    pub total_leader_fee: i64,
    pub foreign_indexes: String,
    pub signature: Option<String>,
    pub timestamp: i64,
    pub base_layer_block_height: i64,
    pub base_layer_block_hash: String,
    pub foreign_proposals: String,
    pub created_at: PrimitiveDateTime,
    pub extra_data: Option<String>,
}

impl TryFrom<ParkedBlock> for (consensus_models::Block, Vec<consensus_models::ForeignProposal>) {
    type Error = StorageError;

    fn try_from(value: ParkedBlock) -> Result<Self, Self::Error> {
        let network = value.network.parse().map_err(|_| StorageError::DecodingError {
            operation: "try_convert",
            item: "block",
            details: format!("Block #{} network byte is not a valid Network", value.id),
        })?;
        let block = consensus_models::Block::load(
            deserialize_hex_try_from(&value.block_id)?,
            network,
            deserialize_hex_try_from(&value.parent_block_id)?,
            deserialize_json(&value.justify)?,
            NodeHeight(value.height as u64),
            Epoch(value.epoch as u64),
            ShardGroup::decode_from_u32(value.shard_group as u32).ok_or_else(|| StorageError::DataInconsistency {
                details: format!(
                    "Block at id={} shard_group ({}) is not a valid",
                    value.id, value.shard_group as u32
                ),
            })?,
            PublicKey::from_canonical_bytes(&deserialize_hex(&value.proposed_by)?).map_err(|_| {
                StorageError::DecodingError {
                    operation: "try_convert",
                    item: "block",
                    details: format!("Block #{} proposed_by is malformed", value.id),
                }
            })?,
            deserialize_json(&value.commands)?,
            deserialize_hex_try_from(&value.merkle_root)?,
            value.total_leader_fee as u64,
            false,
            false,
            false,
            deserialize_json(&value.foreign_indexes)?,
            value.signature.map(|val| deserialize_json(&val)).transpose()?,
            value.created_at,
            None,
            value.timestamp as u64,
            value.base_layer_block_height as u64,
            deserialize_hex_try_from(&value.base_layer_block_hash)?,
            value.extra_data.map(|val| deserialize_json(&val)).transpose()?,
        );
        let foreign_proposals = deserialize_json(&value.foreign_proposals)?;
        Ok((block, foreign_proposals))
    }
}
