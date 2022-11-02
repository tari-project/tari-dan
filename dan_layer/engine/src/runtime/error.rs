//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{fmt::Display, io};

use anyhow::anyhow;
use tari_dan_common_types::optional::IsNotFoundError;
use tari_engine_types::{resource::ResourceError, substate::SubstateAddress};
use tari_template_lib::models::{Amount, BucketId, ComponentAddress, ResourceAddress, VaultId};

use crate::{runtime::id_provider::MaxIdsExceeded, state_store::StateStoreError};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("Encoding error: {0}")]
    EncodingError(#[from] io::Error),
    #[error("State DB error: {0}")]
    StateDbError(#[from] anyhow::Error),
    #[error("State storage error: {0}")]
    StateStoreError(#[from] StateStoreError),
    #[error("Substate not found with address '{address}'")]
    SubstateNotFound { address: SubstateAddress },
    #[error("Component not found with address '{address}'")]
    ComponentNotFound { address: ComponentAddress },
    // #[error("Component does not yet exist at address '{address}'")]
    // ComponentDoesNotExistYet { address: ComponentAddress },
    // #[error("Component has been destroyed at address: {address}")]
    // ComponentDestroyed { address: ComponentAddress },
    #[error("Invalid argument {argument}: {reason}")]
    InvalidArgument { argument: &'static str, reason: String },
    #[error("Invalid amount '{amount}': {reason}")]
    InvalidAmount { amount: Amount, reason: String },
    #[error("Illegal runtime state")]
    IllegalRuntimeState,
    #[error("Vault not found with id ({vault_id})")]
    VaultNotFound { vault_id: VaultId },
    #[error("Bucket not found with id {bucket_id}")]
    BucketNotFound { bucket_id: BucketId },
    #[error("Resource not found with address {resource_address}")]
    ResourceNotFound { resource_address: ResourceAddress },
    #[error(transparent)]
    ResourceError(#[from] ResourceError),
    #[error("Bucket {bucket_id} was dropped but was not empty")]
    BucketNotEmpty { bucket_id: BucketId },
    #[error("Named argument {key} was not found")]
    ItemNotOnWorkspace { key: String },
    #[error("Attempted to take the last output but there was no previous instruction output")]
    NoLastInstructionOutput,
    #[error("Workspace already has an item with key '{key}'")]
    WorkspaceItemKeyExists { key: String },
    #[error(transparent)]
    TransactionCommitError(#[from] TransactionCommitError),
    #[error("Transaction generated too many outputs: {0}")]
    TooManyOutputs(#[from] MaxIdsExceeded),
}

impl RuntimeError {
    pub fn state_db_error<T: Display>(err: T) -> Self {
        RuntimeError::StateDbError(anyhow!("{}", err))
    }
}

impl IsNotFoundError for RuntimeError {
    fn is_not_found_error(&self) -> bool {
        matches!(
            self,
            RuntimeError::ComponentNotFound { .. } |
                RuntimeError::VaultNotFound { .. } |
                RuntimeError::BucketNotFound { .. } |
                RuntimeError::ResourceNotFound { .. }
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransactionCommitError {
    #[error("{count} dangling buckets remain after transaction execution")]
    DanglingBuckets { count: usize },
    #[error("{count} dangling items in workspace after transaction execution")]
    WorkspaceNotEmpty { count: usize },
    #[error(transparent)]
    StateStoreError(#[from] StateStoreError),
    #[error("Failed to obtain a state store transaction: {0}")]
    StateStoreTransactionError(anyhow::Error),
}
