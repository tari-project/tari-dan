//  Copyright 2023. The Tari Project
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

use std::sync::Arc;

use log::info;
use tari_dan_common_types::{optional::IsNotFoundError, NodeAddressable};
use tari_engine_types::commit_result::ExecuteResult;
use tari_epoch_manager::{base_layer::EpochManagerError, EpochManager};
use tari_indexer_lib::{substate_scanner::SubstateScanner, transaction_autofiller::TransactionAutofiller};
use tari_transaction::Transaction;
use tari_validator_node_rpc::client::{ValidatorNodeClientFactory, ValidatorNodeRpcClient};

use crate::dry_run::error::DryRunTransactionProcessorError;

const LOG_TARGET: &str = "tari::indexer::dry_run_transaction_processor";

pub struct DryRunTransactionProcessor<TEpochManager, TClientFactory> {
    epoch_manager: TEpochManager,
    client_provider: TClientFactory,
    transaction_autofiller: TransactionAutofiller<TEpochManager, TClientFactory>,
}

impl<TEpochManager, TClientFactory, TAddr> DryRunTransactionProcessor<TEpochManager, TClientFactory>
where
    TAddr: NodeAddressable,
    TEpochManager: EpochManager<TAddr, Error = EpochManagerError>,
    TClientFactory: ValidatorNodeClientFactory<Addr = TAddr>,
    <TClientFactory::Client as ValidatorNodeRpcClient>::Error: IsNotFoundError,
{
    pub fn new(
        epoch_manager: TEpochManager,
        client_provider: TClientFactory,
        substate_scanner: Arc<SubstateScanner<TEpochManager, TClientFactory>>,
    ) -> Self {
        Self {
            epoch_manager,
            client_provider,
            transaction_autofiller: TransactionAutofiller::new(substate_scanner),
        }
    }

    pub async fn process_transaction(
        &self,
        transaction: &Transaction,
    ) -> Result<ExecuteResult, DryRunTransactionProcessorError> {
        info!(target: LOG_TARGET, "process_transaction: {}", transaction.hash());

        // automatically scan the inputs and add all related involved objects
        // note that this operation does not alter the transaction hash
        let autofilled_transaction = self.transaction_autofiller.autofill_transaction(transaction).await?;

        Err(DryRunTransactionProcessorError::UnexpectecError {
            message: "not implemented".to_string(),
        })
    }
}
