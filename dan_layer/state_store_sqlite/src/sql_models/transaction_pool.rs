//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use diesel::{Queryable, QueryableByName};
use tari_dan_storage::{
    consensus_models,
    consensus_models::{Evidence, LeaderFee},
    StorageError,
};
use time::PrimitiveDateTime;

use crate::serialization::{deserialize_hex_try_from, deserialize_json, parse_from_string};

#[derive(Debug, Clone, Queryable)]
pub struct TransactionPoolRecord {
    #[allow(dead_code)]
    pub id: i32,
    pub transaction_id: String,
    pub original_decision: String,
    pub local_decision: Option<String>,
    pub remote_decision: Option<String>,
    pub evidence: Option<String>,
    pub transaction_fee: i64,
    pub leader_fee: Option<String>,
    pub stage: String,
    // TODO: This is the last stage update, but does not reflect the actual stage (which comes from the
    //       transaction_pool_state_updates table). This is kind of a hack to make transaction_pool_count work
    //       and should not given to TransactionPoolRecord::load.
    #[allow(dead_code)]
    pub pending_stage: Option<String>,
    pub is_ready: bool,
    #[allow(dead_code)]
    pub confirm_stage: Option<String>,
    #[allow(dead_code)]
    pub updated_at: PrimitiveDateTime,
    #[allow(dead_code)]
    pub created_at: PrimitiveDateTime,
}

impl TransactionPoolRecord {
    pub fn try_convert(
        self,
        update: Option<TransactionPoolStateUpdate>,
    ) -> Result<consensus_models::TransactionPoolRecord, StorageError> {
        let mut evidence = self
            .evidence
            .as_deref()
            .map(deserialize_json::<Evidence>)
            .transpose()?
            .unwrap_or_default();
        let mut pending_stage = None;
        let mut local_decision = self.local_decision;
        let mut is_ready = self.is_ready;
        let mut remote_decision = self.remote_decision;
        let mut leader_fee = self.leader_fee;
        let mut transaction_fee = self.transaction_fee;

        if let Some(update) = update {
            evidence = deserialize_json(&update.evidence)?;
            is_ready = update.is_ready;
            pending_stage = Some(parse_from_string(&update.stage)?);
            local_decision = Some(update.local_decision);
            remote_decision = update.remote_decision;
            leader_fee = update.leader_fee;
            transaction_fee = update.transaction_fee;
        }

        let remote_decision = remote_decision.as_deref().map(parse_from_string).transpose()?;
        let leader_fee = leader_fee.as_deref().map(deserialize_json::<LeaderFee>).transpose()?;
        let original_decision = parse_from_string(&self.original_decision)?;

        Ok(consensus_models::TransactionPoolRecord::load(
            deserialize_hex_try_from(&self.transaction_id)?,
            evidence,
            transaction_fee as u64,
            leader_fee,
            parse_from_string(&self.stage)?,
            pending_stage,
            original_decision,
            local_decision.as_deref().map(parse_from_string).transpose()?,
            remote_decision,
            is_ready,
        ))
    }
}

#[derive(Debug, Clone, Queryable, QueryableByName)]
#[diesel(table_name = transaction_pool_state_updates)]
pub struct TransactionPoolStateUpdate {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub id: i32,
    #[allow(dead_code)]
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub block_id: String,
    #[allow(dead_code)]
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub block_height: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub transaction_id: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub stage: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub evidence: String,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub is_ready: bool,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub local_decision: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub transaction_fee: i64,
    #[diesel(sql_type = diesel::sql_types::Nullable < diesel::sql_types::Text >)]
    pub leader_fee: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable < diesel::sql_types::Text >)]
    pub remote_decision: Option<String>,
    #[allow(dead_code)]
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub is_applied: bool,
    #[allow(dead_code)]
    #[diesel(sql_type = diesel::sql_types::Timestamp)]
    pub created_at: PrimitiveDateTime,
}
