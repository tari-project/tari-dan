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

use log::info;
use tari_engine_types::commit_result::ExecuteResult;
use tari_epoch_manager::base_layer::EpochManagerHandle;
use tari_transaction::Transaction;

use crate::dry_run::error::DryRunTransactionProcessorError;

const LOG_TARGET: &str = "tari::indexer::dry_run_transaction_processor";

#[derive(Clone, Debug)]
pub struct DryRunTransactionProcessor {
    _epoch_manager: EpochManagerHandle,
}

impl DryRunTransactionProcessor {
    pub fn new(epoch_manager: EpochManagerHandle) -> Self {
        Self {
            _epoch_manager: epoch_manager,
        }
    }

    pub async fn process_transaction(
        &self,
        transaction: &Transaction,
    ) -> Result<ExecuteResult, DryRunTransactionProcessorError> {
        info!(target: LOG_TARGET, "process_transaction: {}", transaction.hash());
        Err(DryRunTransactionProcessorError::UnexpectecError {
            message: "not implemented".to_string(),
        })
    }
}
