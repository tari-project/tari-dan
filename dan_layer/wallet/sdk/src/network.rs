//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_dan_common_types::PayloadId;
use tari_engine_types::{
    commit_result::ExecuteResult,
    substate::{Substate, SubstateAddress},
};
use tari_transaction::Transaction;

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
        is_dry_run: bool,
    ) -> Result<TransactionQueryResult, Self::Error>;
    async fn query_transaction_result(&self, hash: PayloadId) -> Result<TransactionQueryResult, Self::Error>;
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubstateQueryResult {
    pub address: SubstateAddress,
    pub version: u32,
    pub substate: Substate,
    pub created_by_transaction: FixedHash,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionQueryResult {
    pub transaction_hash: FixedHash,
    pub execution_result: Option<ExecuteResult>,
}
