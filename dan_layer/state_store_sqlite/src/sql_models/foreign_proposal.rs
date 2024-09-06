//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use diesel::Queryable;
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{Epoch, NodeHeight, ShardGroup};
use tari_dan_storage::{consensus_models, StorageError};
use tari_utilities::ByteArray;
use time::PrimitiveDateTime;

use crate::{
    serialization::{deserialize_hex, deserialize_hex_try_from, deserialize_json},
    sql_models::QuorumCertificate,
};

#[derive(Debug, Clone, Queryable)]
pub struct ForeignProposal {
    pub id: i32,
    pub block_id: String,
    pub parent_block_id: String,
    pub merkle_root: String,
    pub network: String,
    pub height: i64,
    pub epoch: i64,
    pub shard_group: i32,
    pub proposed_by: String,
    pub qc: String,
    pub command_count: i64,
    pub commands: String,
    pub total_leader_fee: i64,
    pub foreign_indexes: String,
    pub signature: Option<String>,
    pub timestamp: i64,
    pub base_layer_block_height: i64,
    pub base_layer_block_hash: String,
    pub justify_qc_id: String,
    pub block_pledge: String,
    pub proposed_in_block: Option<String>,
    pub proposed_in_block_height: Option<i64>,
    pub status: String,
    pub created_at: PrimitiveDateTime,
    pub extra_data: Option<String>,
}

impl ForeignProposal {
    pub fn try_convert(self, justify_qc: QuorumCertificate) -> Result<consensus_models::ForeignProposal, StorageError> {
        let network = self.network.parse().map_err(|_| StorageError::DecodingError {
            operation: "try_convert",
            item: "block",
            details: format!("Block #{} network byte is not a valid Network", self.id),
        })?;
        let block = consensus_models::Block::load(
            deserialize_hex_try_from(&self.block_id)?,
            network,
            deserialize_hex_try_from(&self.parent_block_id)?,
            // TODO: we dont need this
            deserialize_json(&self.qc)?,
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
            false,
            false,
            false,
            deserialize_json(&self.foreign_indexes)?,
            self.signature.map(|val| deserialize_json(&val)).transpose()?,
            self.created_at,
            None,
            self.timestamp as u64,
            self.base_layer_block_height as u64,
            deserialize_hex_try_from(&self.base_layer_block_hash)?,
            self.extra_data.map(|val| deserialize_json(&val)).transpose()?,
        );

        let status = consensus_models::ForeignProposalStatus::from_str(&self.status)?;

        Ok(consensus_models::ForeignProposal {
            block,
            block_pledge: deserialize_json(&self.block_pledge)?,
            justify_qc: justify_qc.try_into()?,
            proposed_by_block: self
                .proposed_in_block
                .as_deref()
                .map(deserialize_hex_try_from)
                .transpose()?,
            status,
        })
    }
}

#[derive(Debug, Clone, Queryable)]
pub struct ForeignSendCounters {
    pub id: i32,
    pub block_id: String,
    pub counters: String,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<ForeignSendCounters> for consensus_models::ForeignSendCounters {
    type Error = StorageError;

    fn try_from(value: ForeignSendCounters) -> Result<Self, Self::Error> {
        Ok(Self {
            counters: deserialize_json(&value.counters)?,
        })
    }
}

#[derive(Debug, Clone, Queryable)]
pub struct ForeignReceiveCounters {
    pub id: i32,
    pub counters: String,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<ForeignReceiveCounters> for consensus_models::ForeignReceiveCounters {
    type Error = StorageError;

    fn try_from(value: ForeignReceiveCounters) -> Result<Self, Self::Error> {
        Ok(Self {
            counters: deserialize_json(&value.counters)?,
        })
    }
}
