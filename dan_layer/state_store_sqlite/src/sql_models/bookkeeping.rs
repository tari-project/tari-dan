//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::{consensus_models, StorageError};
use time::PrimitiveDateTime;

use crate::serialization::deserialize_hex_try_from;

#[derive(Debug, Clone, Queryable)]
pub struct HighQc {
    pub id: i32,
    pub block_id: String,
    pub qc_id: String,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<HighQc> for consensus_models::HighQc {
    type Error = StorageError;

    fn try_from(value: HighQc) -> Result<Self, Self::Error> {
        Ok(Self {
            block_id: deserialize_hex_try_from(&value.block_id)?,
            qc_id: deserialize_hex_try_from(&value.qc_id)?,
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
