//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tari_dan_storage::consensus_models::Decision;
use tari_engine_types::{
    commit_result::ExecuteResult,
    substate::{Substate, SubstateAddress},
};
use tari_transaction::{SubstateRequirement, Transaction, TransactionId};

#[async_trait]
pub trait WalletNetworkInterface {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn query_substate(
        &self,
        address: &SubstateAddress,
        version: Option<u32>,
        local_search_only: bool,
    ) -> Result<SubstateQueryResult, Self::Error>;

    async fn submit_transaction(
        &self,
        transaction: Transaction,
        required_substates: Vec<SubstateRequirement>,
    ) -> Result<TransactionId, Self::Error>;

    async fn submit_dry_run_transaction(
        &self,
        transaction: Transaction,
        required_substates: Vec<SubstateRequirement>,
    ) -> Result<TransactionQueryResult, Self::Error>;

    async fn query_transaction_result(
        &self,
        transaction_id: TransactionId,
    ) -> Result<TransactionQueryResult, Self::Error>;
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubstateQueryResult {
    pub address: SubstateAddress,
    pub version: u32,
    pub substate: Substate,
    pub created_by_transaction: TransactionId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionQueryResult {
    pub result: TransactionFinalizedResult,
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransactionFinalizedResult {
    Pending,
    Finalized {
        final_decision: Decision,
        execution_result: Option<ExecuteResult>,
        abort_details: Option<String>,
        json_results: Vec<Value>,
    },
}

impl TransactionFinalizedResult {
    pub fn into_execute_result(self) -> Option<ExecuteResult> {
        match self {
            TransactionFinalizedResult::Pending => None,
            TransactionFinalizedResult::Finalized { execution_result, .. } => execution_result,
        }
    }
}
