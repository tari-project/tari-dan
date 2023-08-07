//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_common_types::{Epoch, NodeAddressable};
use tari_dan_storage::{
    consensus_models,
    consensus_models::{BlockId, QuorumDecision},
    StorageError,
};
use time::PrimitiveDateTime;

use crate::{
    error::SqliteStorageError,
    serialization::{deserialize_hex, deserialize_hex_try_from, deserialize_json},
};

#[derive(Debug, Clone, Queryable)]
pub struct Vote {
    pub id: i32,
    pub hash: String,
    pub epoch: i64,
    pub block_id: String,
    pub decision: i32,
    pub sender: String,
    pub signature: String,
    pub merkle_proof: String,
    pub created_at: PrimitiveDateTime,
}

impl<TAddr: NodeAddressable> TryFrom<Vote> for consensus_models::Vote<TAddr> {
    type Error = StorageError;

    fn try_from(value: Vote) -> Result<Self, Self::Error> {
        Ok(Self {
            epoch: Epoch(value.epoch as u64),
            block_id: BlockId::try_from(deserialize_hex(&value.block_id)?).map_err(|e| {
                SqliteStorageError::MalformedDbData {
                    operation: "TryFrom<Vote> block_id",
                    details: e.to_string(),
                }
            })?,

            decision: QuorumDecision::from_u8(u8::try_from(value.decision).map_err(|_| {
                SqliteStorageError::MalformedDbData {
                    operation: "TryFrom<Vote> decision",
                    details: format!("Could not convert {} to u8", value.decision),
                }
            })?)
            .ok_or_else(|| SqliteStorageError::MalformedDbData {
                operation: "TryFrom<Vote> decision",
                details: format!("Could not convert {} to QuorumDecision", value.decision),
            })?,
            sender_leaf_hash: deserialize_hex_try_from(&value.sender)?,
            signature: deserialize_json(&value.signature)?,
            merkle_proof: deserialize_json(&value.merkle_proof)?,
        })
    }
}
