// Copyright 2023. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use tari_dan_app_utilities::transaction_executor::TransactionProcessorError;
use tari_dan_common_types::{Epoch, SubstateAddress};
use tari_engine_types::substate::SubstateId;
use tari_epoch_manager::EpochManagerError;
use tari_indexer_lib::transaction_autofiller::TransactionAutofillerError;
use tari_rpc_framework::RpcStatus;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DryRunTransactionProcessorError {
    #[error(transparent)]
    TransactionAutofillerError(#[from] TransactionAutofillerError),
    #[error("Substate {id} v{version} is DOWN")]
    SubstateDowned { id: SubstateId, version: u32 },
    #[error("EpochManager error: {0}")]
    EpochManager(#[from] EpochManagerError),
    #[error("Rpc error: {0}")]
    RpcRequestFailed(#[from] RpcStatus),
    #[error("TransactionProcessor error: {0}")]
    PayloadProcessor(#[from] TransactionProcessorError),
    #[error(
        "All validators for epoch {epoch} substate address {address} failed to return substate. does_not_exist: \
         {nexist_count}/{committee_size}, substate_down: {err_count}/{committee_size}"
    )]
    AllValidatorsFailedToReturnSubstate {
        address: SubstateAddress,
        epoch: Epoch,
        nexist_count: usize,
        err_count: usize,
        committee_size: usize,
    },
}
