//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tari_dan_common_types::substate_type::SubstateType;
use tari_dan_storage::consensus_models::Decision;
use tari_engine_types::{
    commit_result::ExecuteResult,
    substate::{Substate, SubstateId},
};
use tari_template_abi::TemplateDef;
use tari_template_lib::prelude::TemplateAddress;
use tari_transaction::{SubstateRequirement, Transaction, TransactionId};

#[async_trait]
pub trait WalletNetworkInterface {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn query_substate(
        &self,
        address: &SubstateId,
        version: Option<u32>,
        local_search_only: bool,
    ) -> Result<SubstateQueryResult, Self::Error>;

    async fn list_substates(
        &self,
        filter_by_template: Option<TemplateAddress>,
        filter_by_type: Option<SubstateType>,
        limit: Option<u64>,
        offset: Option<u64>
    ) -> Result<SubstateListResult, Self::Error>;

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

    async fn fetch_template_definition(&self, template_address: TemplateAddress) -> Result<TemplateDef, Self::Error>;
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubstateQueryResult {
    pub address: SubstateId,
    pub version: u32,
    pub substate: Substate,
    pub created_by_transaction: TransactionId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubstateListResult {
    pub substates: Vec<SubstateListItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubstateListItem {
    pub substate_id: SubstateId,
    pub module_name: Option<String>,
    pub version: u32,
    pub template_address: Option<TemplateAddress>,
    pub timestamp: u64,
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
        execution_time: Duration,
        finalized_time: Duration,
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
