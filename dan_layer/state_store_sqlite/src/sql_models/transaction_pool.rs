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
    pub id: i32,
    pub transaction_id: String,
    pub original_decision: String,
    pub local_decision: Option<String>,
    pub remote_decision: Option<String>,
    pub evidence: Option<String>,
    pub remote_evidence: Option<String>,
    pub transaction_fee: Option<i64>,
    pub leader_fee: Option<i64>,
    pub global_exhaust_burn: Option<i64>,
    pub stage: String,
    // TODO: This is the last stage update, but does not reflect the actual stage (which comes from the
    //       transaction_pool_state_updates table). This is kind of a hack to make transaction_pool_count work
    //       and should not given to TransactionPoolRecord::load.
    pub pending_stage: Option<String>,
    pub is_ready: bool,
    pub updated_at: PrimitiveDateTime,
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
        if let Some(update) = update {
            evidence.merge(deserialize_json::<Evidence>(&update.evidence)?);
            is_ready = update.is_ready;
            pending_stage = Some(parse_from_string(&update.stage)?);
            local_decision = update.local_decision;
        }

        if let Some(ref remote_evidence) = self.remote_evidence {
            evidence.merge(deserialize_json::<Evidence>(remote_evidence)?);
        }

        let leader_fee = self
            .leader_fee
            .map(|leader_fee| -> Result<LeaderFee, StorageError> {
                Ok(LeaderFee {
                    fee: leader_fee as u64,
                    global_exhaust_burn: self.global_exhaust_burn.map(|burn| burn as u64).ok_or_else(|| {
                        StorageError::DataInconsistency {
                            details: format!(
                                "TransactionPoolRecord {} has a leader_fee but no global_exhaust_burn",
                                self.id
                            ),
                        }
                    })?,
                })
            })
            .transpose()?;
        let original_decision = parse_from_string(&self.original_decision)?;
        let remote_decision = self.remote_decision.as_deref().map(parse_from_string).transpose()?;

        Ok(consensus_models::TransactionPoolRecord::load(
            deserialize_hex_try_from(&self.transaction_id)?,
            evidence,
            self.transaction_fee.map(|f| f as u64),
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
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub block_id: String,
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
    #[diesel(sql_type = diesel::sql_types::Nullable < diesel::sql_types::Text >)]
    pub local_decision: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Timestamp)]
    pub created_at: PrimitiveDateTime,
}
