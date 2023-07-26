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

use std::fmt::Display;

use anyhow::anyhow;
use tari_bor::BorError;
use tari_dan_common_types::optional::IsNotFoundError;
use tari_engine_types::{
    resource_container::ResourceError,
    substate::SubstateAddress,
    transaction_receipt::TransactionReceiptAddress,
};
use tari_template_lib::models::{
    Amount,
    BucketId,
    ComponentAddress,
    NonFungibleId,
    ResourceAddress,
    TemplateAddress,
    UnclaimedConfidentialOutputAddress,
    VaultId,
};
use tari_transaction::id_provider::IdProviderError;

use super::workspace::WorkspaceError;
use crate::{
    runtime::{FunctionIdent, RuntimeModuleError},
    state_store::StateStoreError,
};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("Runtime encoding error: {0}")]
    EncodingError(#[from] BorError),
    #[error("State DB error: {0}")]
    StateDbError(#[from] anyhow::Error),
    #[error("State storage error: {0}")]
    StateStoreError(#[from] StateStoreError),
    #[error("Workspace error: {0}")]
    WorkspaceError(#[from] WorkspaceError),
    #[error("Substate not found with address '{address}'")]
    SubstateNotFound { address: SubstateAddress },
    #[error("Component not found with address '{address}'")]
    ComponentNotFound { address: ComponentAddress },
    #[error("Layer one commitment not found with address '{address}'")]
    LayerOneCommitmentNotFound {
        address: UnclaimedConfidentialOutputAddress,
    },
    #[error("Invalid argument {argument}: {reason}")]
    InvalidArgument { argument: &'static str, reason: String },
    #[error("Invalid amount '{amount}': {reason}")]
    InvalidAmount { amount: Amount, reason: String },
    #[error("Illegal runtime state")]
    IllegalRuntimeState,
    #[error("Vault not found with id ({vault_id})")]
    VaultNotFound { vault_id: VaultId },
    #[error("Non-fungible token not found with address {resource_address} and id {nft_id}")]
    NonFungibleNotFound {
        resource_address: ResourceAddress,
        nft_id: NonFungibleId,
    },
    #[error("Invalid op '{op}' on burnt non-fungible {resource_address} id {nf_id}")]
    InvalidOpNonFungibleBurnt {
        op: &'static str,
        resource_address: ResourceAddress,
        nf_id: NonFungibleId,
    },
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
    #[error(transparent)]
    TransactionCommitError(#[from] TransactionCommitError),
    #[error("Transaction generated too many outputs: {0}")]
    TooManyOutputs(#[from] IdProviderError),
    #[error("Duplicate NFT token id: {token_id}")]
    DuplicateNonFungibleId { token_id: NonFungibleId },
    #[error("Access Denied: {fn_ident}")]
    AccessDenied { fn_ident: FunctionIdent },
    #[error("Invalid method address rule for {template_name}: {details}")]
    InvalidMethodAccessRule { template_name: String, details: String },
    #[error("Runtime module error: {0}")]
    ModuleError(#[from] RuntimeModuleError),
    #[error("Invalid claiming signature")]
    InvalidClaimingSignature,
    #[error("Invalid range proof")]
    InvalidRangeProof,
    #[error("Invalid substate type")]
    InvalidSubstateType,
    #[error("Layer one commitment already claimed with address '{address}'")]
    ConfidentialOutputAlreadyClaimed {
        address: UnclaimedConfidentialOutputAddress,
    },
    #[error("Template {template_address} not found")]
    TemplateNotFound { template_address: TemplateAddress },
    #[error("Insufficient fees paid: required {required_fee}, paid {fees_paid}")]
    InsufficientFeesPaid { required_fee: Amount, fees_paid: Amount },
    #[error("No checkpoint")]
    NoCheckpoint,
    #[error("Component address must be sequential. Index before {index} was not found")]
    ComponentAddressMustBeSequential { index: u32 },
    #[error("Failed to load template '{address}': {details}")]
    FailedToLoadTemplate { address: TemplateAddress, details: String },
    #[error("Transaction Receipt already exists {address}")]
    TransactionReceiptAlreadyExists { address: TransactionReceiptAddress },
    #[error("Transaction Receipt not found")]
    TransactionReceiptNotFound,
    #[error("Component already exists {address}")]
    ComponentAlreadyExists { address: ComponentAddress },
    #[error("Call function error of function '{function}' on template '{template_address}': {details}")]
    CallFunctionError {
        template_address: TemplateAddress,
        function: String,
        details: String,
    },
    #[error("Call method error of method '{method}' on component '{component_address}': {details}")]
    CallMethodError {
        component_address: ComponentAddress,
        method: String,
        details: String,
    },
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
                RuntimeError::ResourceNotFound { .. } |
                RuntimeError::NonFungibleNotFound { .. }
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
    #[error(transparent)]
    IdProviderError(#[from] IdProviderError),
    #[error("trying to mutate non fungible index of resource {resource_address} at index {index}")]
    NonFungibleIndexMutation {
        resource_address: ResourceAddress,
        index: u64,
    },
}
