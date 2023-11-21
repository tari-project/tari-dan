//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use log::info;
use tari_common_types::types::PublicKey;
use tari_comms::protocol::rpc::RpcStatus;
use tari_dan_app_utilities::{
    template_manager::implementation::TemplateManager,
    transaction_executor::{TariDanTransactionProcessor, TransactionExecutor, TransactionProcessorError}, substate_file_cache::SubstateFileCache,
};
use tari_dan_engine::{
    bootstrap_state,
    state_store::{memory::MemoryStateStore, AtomicDb, StateStoreError, StateWriter},
};
use tari_dan_storage::StorageError;
use tari_engine_types::commit_result::ExecuteResult;
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerError, EpochManagerReader};
use tari_state_store_sqlite::SqliteStateStore;
use tari_transaction::Transaction;
use tari_validator_node_client::ValidatorNodeClientError;
use tari_validator_node_rpc::client::TariCommsValidatorNodeClientFactory;
use thiserror::Error;
use tokio::task;

use crate::{
    p2p::services::mempool::SubstateResolver,
    substate_resolver::{SubstateResolverError, TariSubstateResolver},
    virtual_substate::VirtualSubstateError,
};

const LOG_TARGET: &str = "tari::dan::validator_node::dry_run_transaction_processor";

#[derive(Error, Debug)]
pub enum DryRunTransactionProcessorError {
    #[error("PayloadProcessor error: {0}")]
    PayloadProcessor(#[from] TransactionProcessorError),
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("EpochManager error: {0}")]
    EpochManager(#[from] EpochManagerError),
    #[error("Validator node client error: {0}")]
    ValidatorNodeClient(#[from] ValidatorNodeClientError),
    #[error("Rpc error: {0}")]
    RpcRequestFailed(#[from] RpcStatus),
    #[error("State store error: {0}")]
    StateStoreError(#[from] StateStoreError),
    #[error("Substate resolver error: {0}")]
    SubstateResoverError(#[from] SubstateResolverError),
    #[error("Virtual substate error: {0}")]
    VirtualSubstateError(#[from] VirtualSubstateError),
}

#[derive(Clone, Debug)]
pub struct DryRunTransactionProcessor {
    substate_resolver:
        TariSubstateResolver<SqliteStateStore<PublicKey>, EpochManagerHandle, TariCommsValidatorNodeClientFactory, SubstateFileCache>,
    epoch_manager: EpochManagerHandle,
    payload_processor: TariDanTransactionProcessor<TemplateManager>,
}

impl DryRunTransactionProcessor {
    pub fn new(
        epoch_manager: EpochManagerHandle,
        payload_processor: TariDanTransactionProcessor<TemplateManager>,
        substate_resolver: TariSubstateResolver<
            SqliteStateStore<PublicKey>,
            EpochManagerHandle,
            TariCommsValidatorNodeClientFactory,
            SubstateFileCache,
        >,
    ) -> Self {
        Self {
            substate_resolver,
            epoch_manager,
            payload_processor,
        }
    }

    pub async fn process_transaction(
        &self,
        transaction: Transaction,
    ) -> Result<ExecuteResult, DryRunTransactionProcessorError> {
        // Resolve all local and foreign substates
        let temp_state_store = MemoryStateStore::new();
        {
            let mut tx = temp_state_store.write_access().map_err(StateStoreError::Custom)?;
            bootstrap_state(&mut tx)?;
            tx.commit()?;
        }

        let current_epoch = self.epoch_manager.current_epoch().await?;
        let virtual_substates = self
            .substate_resolver
            .resolve_virtual_substates(&transaction, current_epoch)
            .await?;

        self.substate_resolver.resolve(&transaction, &temp_state_store).await?;

        // execute the payload in the WASM engine and return the result
        let executed = task::block_in_place(|| {
            self.payload_processor
                .execute(transaction, temp_state_store, virtual_substates)
        })?;
        let result = executed.into_result();

        if let Some(ref fees) = result.fee_receipt {
            info!(target: LOG_TARGET, "Transaction fees: {}", fees.total_fees_charged());
        }

        Ok(result)
    }
}
