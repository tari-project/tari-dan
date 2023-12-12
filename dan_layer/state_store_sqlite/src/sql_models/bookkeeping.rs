//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_common_types::{shard_bucket::ShardBucket, Epoch, NodeAddressable, NodeHeight};
use tari_dan_storage::{
    consensus_models::{self, QuorumDecision},
    StorageError,
};
use time::PrimitiveDateTime;

use crate::{
    error::SqliteStorageError,
    serialization::{deserialize_hex_try_from, deserialize_json, parse_from_string},
};

#[derive(Debug, Clone, Queryable)]
pub struct HighQc {
    pub id: i32,
    pub block_id: String,
    pub block_height: i64,
    pub qc_id: String,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<HighQc> for consensus_models::HighQc {
    type Error = StorageError;

    fn try_from(value: HighQc) -> Result<Self, Self::Error> {
        Ok(Self {
            block_id: deserialize_hex_try_from(&value.block_id)?,
            block_height: NodeHeight(value.block_height as u64),
            qc_id: deserialize_hex_try_from(&value.qc_id)?,
        })
    }
}

#[derive(Debug, Clone, Queryable)]
pub struct ForeignProposal {
    pub id: i32,
    pub bucket: i32,
    pub block_id: String,
    pub state: String,
    pub mined_at: Option<i64>,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<ForeignProposal> for consensus_models::ForeignProposal {
    type Error = StorageError;

    fn try_from(value: ForeignProposal) -> Result<Self, Self::Error> {
        Ok(Self {
            bucket: ShardBucket::from(value.bucket as u32),
            block_id: deserialize_hex_try_from(&value.block_id)?,
            state: parse_from_string(&value.state)?,
            mined_at: value.mined_at.map(|mined_at| NodeHeight(mined_at as u64)),
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

#[derive(Debug, Clone, Queryable)]
pub struct LockedBlock {
    pub id: i32,
    pub block_id: String,
    pub height: i64,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<LockedBlock> for consensus_models::LockedBlock {
    type Error = StorageError;

    fn try_from(value: LockedBlock) -> Result<Self, Self::Error> {
        Ok(Self {
            block_id: deserialize_hex_try_from(&value.block_id)?,
            height: NodeHeight(value.height as u64),
        })
    }
}

#[derive(Debug, Clone, Queryable)]
pub struct LastExecuted {
    pub id: i32,
    pub block_id: String,
    pub height: i64,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<LastExecuted> for consensus_models::LastExecuted {
    type Error = StorageError;

    fn try_from(value: LastExecuted) -> Result<Self, Self::Error> {
        Ok(Self {
            block_id: deserialize_hex_try_from(&value.block_id)?,
            height: NodeHeight(value.height as u64),
        })
    }
}

#[derive(Debug, Clone, Queryable)]
pub struct LastVoted {
    pub id: i32,
    pub block_id: String,
    pub height: i64,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<LastVoted> for consensus_models::LastVoted {
    type Error = StorageError;

    fn try_from(value: LastVoted) -> Result<Self, Self::Error> {
        Ok(Self {
            block_id: deserialize_hex_try_from(&value.block_id)?,
            height: NodeHeight(value.height as u64),
        })
    }
}

#[derive(Debug, Clone, Queryable)]
pub struct LastSentVote {
    pub id: i32,
    pub epoch: i64,
    pub block_id: String,
    pub block_height: i64,
    pub decision: i32,
    pub signature: String,
    pub created_at: PrimitiveDateTime,
}

impl<TAddr: NodeAddressable> TryFrom<LastSentVote> for consensus_models::LastSentVote<TAddr> {
    type Error = StorageError;

    fn try_from(value: LastSentVote) -> Result<Self, Self::Error> {
        Ok(Self {
            epoch: Epoch(value.epoch as u64),
            block_id: deserialize_hex_try_from(&value.block_id)?,
            block_height: NodeHeight(value.block_height as u64),
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
            signature: deserialize_json(&value.signature)?,
        })
    }
}

#[derive(Debug, Clone, Queryable)]
pub struct LastProposed {
    pub id: i32,
    pub block_id: String,
    pub height: i64,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<LastProposed> for consensus_models::LastProposed {
    type Error = StorageError;

    fn try_from(value: LastProposed) -> Result<Self, Self::Error> {
        Ok(Self {
            block_id: deserialize_hex_try_from(&value.block_id)?,
            height: NodeHeight(value.height as u64),
        })
    }
}
