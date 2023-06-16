//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use axum::async_trait;
use reqwest::{IntoUrl, Url};
use tari_dan_common_types::{
    optional::{IsNotFoundError, Optional},
    PayloadId,
};
use tari_dan_wallet_sdk::network::{SubstateQueryResult, TransactionResult, WalletNetworkInterface};
use tari_engine_types::substate::SubstateAddress;
use tari_indexer_client::{
    error::IndexerClientError,
    json_rpc_client::IndexerJsonRpcClient,
    types::{GetSubstateRequest, GetTransactionResultRequest, SubmitTransactionRequest},
};
use tari_transaction::Transaction;

#[derive(Debug, Clone)]
pub struct IndexerJsonRpcNetworkInterface {
    indexer_jrpc_address: Url,
}

impl IndexerJsonRpcNetworkInterface {
    pub fn new<T: IntoUrl>(indexer_jrpc_address: T) -> Self {
        Self {
            indexer_jrpc_address: indexer_jrpc_address
                .into_url()
                .expect("Malformed indexer JSON-RPC address"),
        }
    }

    fn get_client(&self) -> Result<IndexerJsonRpcClient, IndexerJrpcError> {
        let client = IndexerJsonRpcClient::connect(self.indexer_jrpc_address.clone())?;
        Ok(client)
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
        is_dry_run: bool,
    ) -> Result<TransactionResult, Self::Error> {
        let mut client = self.get_client()?;
        let result = client
            .submit_transaction(SubmitTransactionRequest {
                transaction,
                is_dry_run,
            })
            .await?;
        Ok(TransactionResult {
            transaction_hash: result.transaction_hash,
            execution_result: result.execution_result,
        })
    }

    async fn query_transaction_result(&self, transaction_hash: PayloadId) -> Result<TransactionResult, Self::Error> {
        let mut client = self.get_client()?;
        let maybe_result = client
            .get_transaction_result(GetTransactionResultRequest { transaction_hash })
            .await
            .optional()?;

        let Some(result) = maybe_result else {
            return Ok(TransactionResult {
                execution_result: None,
                transaction_hash: transaction_hash.into_array().into(),
            });
        };

        Ok(TransactionResult {
            execution_result: result.execution_result,
            transaction_hash: transaction_hash.into_array().into(),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IndexerJrpcError {
    #[error("Indexer client error: {0}")]
    IndexerClientError(#[from] IndexerClientError),
}

impl IsNotFoundError for IndexerJrpcError {
    fn is_not_found_error(&self) -> bool {
        match self {
            IndexerJrpcError::IndexerClientError(err) => err.is_not_found_error(),
        }
    }
}
