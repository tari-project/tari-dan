//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::{Arc, Mutex};

use axum::async_trait;
use reqwest::{IntoUrl, Url};
use tari_dan_common_types::optional::IsNotFoundError;
use tari_dan_wallet_sdk::network::{
    SubstateQueryResult,
    TransactionFinalizedResult,
    TransactionQueryResult,
    WalletNetworkInterface,
};
use tari_engine_types::substate::SubstateAddress;
use tari_indexer_client::{
    error::IndexerClientError,
    json_rpc_client::IndexerJsonRpcClient,
    types::{
        GetSubstateRequest,
        GetTransactionResultRequest,
        IndexerTransactionFinalizedResult,
        SubmitTransactionRequest,
    },
};
use tari_transaction::{SubstateRequirement, Transaction, TransactionId};
use url::ParseError;

#[derive(Debug, Clone)]
pub struct IndexerJsonRpcNetworkInterface {
    indexer_jrpc_address: Arc<Mutex<Url>>,
}

impl IndexerJsonRpcNetworkInterface {
    pub fn new<T: IntoUrl>(indexer_jrpc_address: T) -> Self {
        Self {
            indexer_jrpc_address: Arc::new(Mutex::new(
                indexer_jrpc_address
                    .into_url()
                    .expect("Malformed indexer JSON-RPC address"),
            )),
        }
    }

    fn get_client(&self) -> Result<IndexerJsonRpcClient, IndexerJrpcError> {
        let client = IndexerJsonRpcClient::connect((*self.indexer_jrpc_address.lock().unwrap()).clone())?;
        Ok(client)
    }

    pub fn set_endpoint(&mut self, endpoint: &str) -> Result<(), IndexerJrpcError> {
        *self.indexer_jrpc_address.lock().unwrap() = Url::parse(endpoint)?;
        Ok(())
    }

    pub fn get_endpoint(&self) -> Url {
        (*self.indexer_jrpc_address.lock().unwrap()).clone()
    }
}

#[async_trait]
impl WalletNetworkInterface for IndexerJsonRpcNetworkInterface {
    type Error = IndexerJrpcError;

    async fn query_substate(
        &self,
        address: &SubstateAddress,
        version: Option<u32>,
        local_search_only: bool,
    ) -> Result<SubstateQueryResult, Self::Error> {
        let mut client = self.get_client()?;
        let result = client
            .get_substate(GetSubstateRequest {
                address: address.clone(),
                version,
                local_search_only,
            })
            .await?;
        Ok(SubstateQueryResult {
            address: result.address,
            version: result.version,
            substate: result.substate,
            created_by_transaction: result.created_by_transaction,
        })
    }

    async fn submit_transaction(
        &self,
        transaction: Transaction,
        required_substates: Vec<SubstateRequirement>,
    ) -> Result<TransactionId, Self::Error> {
        let mut client = self.get_client()?;
        let result = client
            .submit_transaction(SubmitTransactionRequest {
                transaction,
                required_substates,
                is_dry_run: false,
            })
            .await?;
        Ok(result.transaction_id)
    }

    async fn submit_dry_run_transaction(
        &self,
        transaction: Transaction,
        required_substates: Vec<SubstateRequirement>,
    ) -> Result<TransactionQueryResult, Self::Error> {
        let mut client = self.get_client()?;
        let resp = client
            .submit_transaction(SubmitTransactionRequest {
                transaction,
                required_substates,
                is_dry_run: true,
            })
            .await?;

        Ok(TransactionQueryResult {
            transaction_id: resp.transaction_id,
            result: convert_indexer_result_to_wallet_result(resp.result),
        })
    }

    async fn query_transaction_result(
        &self,
        transaction_id: TransactionId,
    ) -> Result<TransactionQueryResult, Self::Error> {
        let mut client = self.get_client()?;
        let resp = client
            .get_transaction_result(GetTransactionResultRequest { transaction_id })
            .await?;

        Ok(TransactionQueryResult {
            transaction_id,
            result: convert_indexer_result_to_wallet_result(resp.result),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IndexerJrpcError {
    #[error("Indexer client error: {0}")]
    IndexerClientError(#[from] IndexerClientError),
    #[error("Indexer parse error : {0}")]
    IndexerParseError(#[from] ParseError),
}

impl IsNotFoundError for IndexerJrpcError {
    fn is_not_found_error(&self) -> bool {
        match self {
            IndexerJrpcError::IndexerClientError(err) => err.is_not_found_error(),
            _ => false,
        }
    }
}

/// These types are identical, however in order to keep the wallet decoupled from the indexer, we define two types and
/// this conversion function.
// TODO: the common interface and types between the wallet and indexer could be made into a shared "view of the network"
// interface and we can avoid defining two types.
fn convert_indexer_result_to_wallet_result(result: IndexerTransactionFinalizedResult) -> TransactionFinalizedResult {
    match result {
        IndexerTransactionFinalizedResult::Pending => TransactionFinalizedResult::Pending,
        IndexerTransactionFinalizedResult::Finalized {
            final_decision,
            execution_result,
            abort_details,
            json_results,
        } => TransactionFinalizedResult::Finalized {
            final_decision,
            execution_result,
            abort_details,
            json_results,
        },
    }
}
