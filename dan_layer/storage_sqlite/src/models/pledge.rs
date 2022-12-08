//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use chrono::NaiveDateTime;

use crate::schema::*;

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
#[table_name = "shard_pledges"]
pub struct NewShardPledge {
    pub shard_id: Vec<u8>,
    pub created_height: i64,
    pub pledged_to_payload_id: Vec<u8>,
    pub is_active: bool,
}
