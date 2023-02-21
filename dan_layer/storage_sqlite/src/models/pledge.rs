//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::convert::{TryFrom, TryInto};

use chrono::NaiveDateTime;
use tari_dan_common_types::{ObjectPledgeInfo, ShardId};

use crate::{error::SqliteStorageError, schema::*};

#[derive(Debug, Identifiable, Queryable)]
pub struct ShardPledge {
    pub id: i32,
    pub shard_id: Vec<u8>,
    pub created_height: i64,
    pub pledged_to_payload_id: Vec<u8>,
    pub is_active: bool,
    pub completed_by_tree_node_hash: Option<Vec<u8>>,
    pub abandoned_by_tree_node_hash: Option<Vec<u8>>,
    pub timestamp: NaiveDateTime,
    pub updated_timestamp: Option<NaiveDateTime>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = shard_pledges)]
pub struct NewShardPledge {
    pub shard_id: Vec<u8>,
    pub created_height: i64,
    pub pledged_to_payload_id: Vec<u8>,
    pub is_active: bool,
}

impl TryFrom<ShardPledge> for ObjectPledgeInfo {
    type Error = SqliteStorageError;

    fn try_from(value: ShardPledge) -> Result<Self, Self::Error> {
        Ok(Self {
            shard_id: ShardId::try_from(value.shard_id)
                .map_err(|_| SqliteStorageError::MalformedDbData("malformed shard ID in object pledge".to_string()))?,
            pledged_to_payload_id: value.pledged_to_payload_id.try_into().map_err(|_| {
                SqliteStorageError::MalformedDbData("malformed payload ID in object pledge".to_string())
            })?,
            completed_by_tree_node_hash: value
                .completed_by_tree_node_hash
                .map(TryInto::try_into)
                .transpose()
                .map_err(|_| {
                    SqliteStorageError::MalformedDbData(
                        "malformed completed_by_tree_node_hash in object pledge".to_string(),
                    )
                })?,
            abandoned_by_tree_node_hash: value
                .abandoned_by_tree_node_hash
                .map(TryInto::try_into)
                .transpose()
                .map_err(|_| {
                    SqliteStorageError::MalformedDbData(
                        "malformed abandoned_by_tree_node_hash in object pledge".to_string(),
                    )
                })?,
            is_active: value.is_active,
        })
    }
}
