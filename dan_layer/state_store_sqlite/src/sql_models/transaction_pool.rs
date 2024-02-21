//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use diesel::{Queryable, QueryableByName};
use tari_dan_storage::{
    consensus_models,
    consensus_models::{Evidence, TransactionAtom},
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
    pub evidence: String,
    pub remote_evidence: Option<String>,
    pub transaction_fee: i64,
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
        mut self,
        update: Option<TransactionPoolStateUpdate>,
    ) -> Result<consensus_models::TransactionPoolRecord, StorageError> {
        let mut evidence = deserialize_json::<Evidence>(&self.evidence)?;
        let mut pending_stage = None;
        if let Some(update) = update {
            evidence.merge(deserialize_json::<Evidence>(&update.evidence)?);
            self.is_ready = update.is_ready;
            pending_stage = Some(parse_from_string(&update.stage)?);
        }

        if let Some(ref remote_evidence) = self.remote_evidence {
            evidence.merge(deserialize_json::<Evidence>(remote_evidence)?);
        }

        Ok(consensus_models::TransactionPoolRecord::load(
            TransactionAtom {
                id: deserialize_hex_try_from(&self.transaction_id)?,
                decision: parse_from_string(&self.original_decision)?,
                evidence,
                transaction_fee: self.transaction_fee as u64,
            },
            parse_from_string(&self.stage)?,
            pending_stage,
            self.local_decision.as_deref().map(parse_from_string).transpose()?,
            self.remote_decision.as_deref().map(parse_from_string).transpose()?,
            self.is_ready,
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
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub local_decision: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Timestamp)]
    pub created_at: PrimitiveDateTime,
}
