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

use tari_bor::BorError;
use tari_engine_types::{
    commit_result::RejectReason,
    entity_id_provider::EntityIdProviderError,
    id_provider::IdProviderError,
    indexed_value::IndexedValueError,
    limits,
    lock::LockId,
    resource_container::ResourceError,
    substate::SubstateId,
    virtual_substate::VirtualSubstateId,
};
use tari_ootle_common_types::{displayable::Displayable, optional::IsNotFoundError};
use tari_ootle_transaction::{
    CheckOrd,
    NftCheck,
    args::{WorkspaceId, WorkspaceOffsetId},
};
use tari_template_abi::EngineOp;
use tari_template_lib::{
    args::{CallAction, VaultFreezeFlag},
    models::{AddressAllocationId, BucketId, ProofId},
    prelude::RistrettoPublicKeyBytes,
    types::{
        Amount,
        ClaimedOutputTombstoneAddress,
        ComponentAddress,
        NonFungibleId,
        ResourceAddress,
        ResourceType,
        TemplateAddress,
        TransactionReceiptAddress,
        VaultId,
        crypto::PedersenCommitmentBytes,
        engine_args::IntrinsicId,
    },
};

use super::workspace::WorkspaceError;
use crate::{
    runtime::{ActionIdent, RuntimeModuleError, locking::LockError},
    state_store::StateStoreError,
};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("Encoding error: {0}")]
    EncodingError(#[from] BorError),
    #[error("Indexed value error: {0}")]
    IndexedValueError(#[from] IndexedValueError),
    #[error("State storage error: {0}")]
    StateStoreError(#[from] StateStoreError),
    #[error("Workspace error: {0}")]
    WorkspaceError(#[from] WorkspaceError),
    #[error("Substate '{id}' not found or is not a transaction input")]
    SubstateNotFound { id: SubstateId },
    #[error("Root substate '{id}' not found")]
    RootSubstateNotFound { id: SubstateId },
    #[error("Referenced substate '{id}' not found")]
    ReferencedSubstateNotFound { id: SubstateId },
    #[error("Non-existent substate '{id}' returned from call")]
    NonExistentSubstateReturned { id: SubstateId },
    #[error("Substate '{id}' not in scope")]
    SubstateOutOfScope { id: SubstateId },
    #[error("Substate {id} is not owned by {requested_owner}")]
    SubstateNotOwned {
        id: SubstateId,
        // To reduce the size of this variant, we box one of the fields
        requested_owner: Box<SubstateId>,
    },
    #[error("Expected lock {lock_id} to lock {expected_type} but it locks {id}")]
    LockSubstateMismatch {
        lock_id: LockId,
        expected_type: &'static str,
        id: SubstateId,
    },
    #[error("Component {component} referenced an unknown substate {id}")]
    ComponentReferencedUnknownSubstate {
        component: ComponentAddress,
        id: SubstateId,
    },
    #[error("Encountered unknown or out of scope bucket {bucket_id}")]
    BucketNotInScope { bucket_id: BucketId },
    #[error("Encountered unknown or out of scope proof {proof_id}")]
    ProofNotInScope { proof_id: ProofId },
    #[error("Encountered unknown or out of scope address allocation {id}")]
    AddressAllocationNotInScope { id: AddressAllocationId },
    #[error("Encountered unknown or out of scope signer badge with public key {public_key}")]
    SignerBadgeNotInScope { public_key: RistrettoPublicKeyBytes },
    #[error("Component not found with address '{address}'")]
    ComponentNotFound { address: ComponentAddress },
    #[error("Layer one commitment not found with address '{address}'")]
    LayerOneCommitmentNotFound { address: ClaimedOutputTombstoneAddress },
    #[error("Invalid argument {argument}: {reason}")]
    InvalidArgument { argument: &'static str, reason: String },
    #[error(
        "Intrinsic {intrinsic} is not supported by this validator. The template requires a newer engine — validators \
         and indexers must be upgraded before it can be called."
    )]
    IntrinsicNotSupported { intrinsic: IntrinsicId },
    #[error("Invalid number of arguments: expected {expected}, but got {len}")]
    InvalidNumberOfArguments { expected: usize, len: usize },
    #[error("Invalid amount '{amount}': {reason}")]
    InvalidAmount { amount: Amount, reason: String },
    #[error("Call frame error: {details}")]
    CurrentFrameError { details: String },
    #[error("Vault not found with id ({vault_id})")]
    VaultNotFound { vault_id: VaultId },
    #[error("Vault {vault_id} is frozen for {freeze_flag}")]
    VaultFrozen {
        vault_id: VaultId,
        freeze_flag: VaultFreezeFlag,
    },
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
    #[error("Proof not found with id {proof_id}")]
    ProofNotFound { proof_id: ProofId },
    #[error("Resource not found with address {resource_address}")]
    ResourceNotFound { resource_address: ResourceAddress },
    #[error(transparent)]
    ResourceError(#[from] ResourceError),
    #[error(
        "Resource supply would overflow: resource {resource_address}, current supply {current_supply}, amount {amount}"
    )]
    ResourceSupplyWouldOverflow {
        resource_address: ResourceAddress,
        current_supply: Amount,
        amount: Amount,
    },
    #[error(
        "Resource supply would underflow: resource {resource_address}, current supply {current_supply}, amount \
         {amount}"
    )]
    ResourceSupplyWouldUnderflow {
        resource_address: ResourceAddress,
        current_supply: Amount,
        amount: Amount,
    },
    #[error(
        "Commitment {commitment} of resource {resource_address} requires a value proof because the resource tracks \
         total supply"
    )]
    MissingValueProofForCommitment {
        resource_address: ResourceAddress,
        commitment: PedersenCommitmentBytes,
    },
    #[error(
        "The amounts being minted or destroyed for resource {resource_address} sum to more than the maximum amount"
    )]
    ValueSumOverflow { resource_address: ResourceAddress },
    #[error("Bucket {bucket_id} was dropped but was not empty")]
    BucketNotEmpty { bucket_id: BucketId },
    #[error("Item at id {id} does not exist on the workspace (existing ids: {})", .existing_ids.display())]
    ItemNotOnWorkspace {
        id: WorkspaceOffsetId,
        existing_ids: Vec<WorkspaceId>,
    },
    #[error("Attempted to take the last output but there was no previous instruction output")]
    NoLastInstructionOutput,
    #[error(transparent)]
    TransactionCommitError(#[from] TransactionCommitError),
    #[error("Transaction generated too many outputs: {0}")]
    TooManyOutputs(#[from] IdProviderError),
    #[error("Transaction generated too many new entities: {0}")]
    TooManyEntities(#[from] EntityIdProviderError),
    #[error("Duplicate NFT token id: {token_id}")]
    DuplicateNonFungibleId { token_id: NonFungibleId },
    #[error("Access Denied: {action_ident}")]
    AccessDenied { action_ident: ActionIdent },
    #[error("Access Denied: attempt to set state on component {attempted_on} from another component {attempted_by}")]
    AccessDeniedSetComponentState {
        attempted_on: SubstateId,
        // To reduce the size of this variant, we box one of the fields
        attempted_by: Box<SubstateId>,
    },
    #[error("Resource Auth Hook Denied Access for action {action_ident}: {details}")]
    AccessDeniedAuthHook { action_ident: ActionIdent, details: String },
    #[error("Access Denied: You must be the owner to perform this action: {action}")]
    AccessDeniedOwnerRequired { action: ActionIdent },
    #[error("Write attempted in a read-only execution context")]
    WriteInReadOnlyContext,
    #[error("Host operation '{operation}' is forbidden inside a read-only (spend-script) context")]
    ForbiddenInReadOnlyContext { operation: &'static str },
    #[error("Write to {id} attempted in a resource auth hook, which may only modify its own component state")]
    WriteOutsideOwnComponent { id: SubstateId },
    #[error("Host operation '{operation}' is forbidden inside a resource auth hook")]
    ForbiddenInAuthHookContext { operation: &'static str },
    #[error("Recall on resource {resource_address} targeted vault {vault_id}, which holds resource {vault_resource}")]
    RecallResourceMismatch {
        vault_id: VaultId,
        resource_address: ResourceAddress,
        vault_resource: ResourceAddress,
    },
    #[error("Spend script rejected the spend: {details}")]
    SpendScriptRejected { details: Box<RuntimeError> },
    #[error("Spend condition not met: {details}")]
    SpendConditionNotMet { details: String },
    #[error("Spend context is only available during spend-script execution")]
    SpendContextUnavailable,
    #[error("Invalid method address rule for {template_name}: {details}")]
    InvalidMethodAccessRule { template_name: String, details: String },
    #[error("Runtime module error: {0}")]
    ModuleError(#[from] RuntimeModuleError),
    #[error("Invalid burn claim proof: {details}")]
    InvalidClaimProof { details: String },
    #[error("Layer one commitment already claimed with address '{address}'")]
    ConfidentialOutputAlreadyClaimed { address: ClaimedOutputTombstoneAddress },
    #[error("Template {template_address} not found")]
    TemplateNotFound { template_address: TemplateAddress },
    #[error("Insufficient fees paid: required {required_fee}, paid {fees_paid}")]
    InsufficientFeesPaid { required_fee: u64, fees_paid: u64 },
    #[error(
        "Native verification requiring {required_points} points exceeds the fee intent's {credit_points} points of \
         compute credit: {consumed_points} already consumed. The credit is a flat allowance for sourcing a fee and \
         does not rise with the fee paid — move this work to the main instructions."
    )]
    FeeIntentComputeExceeded {
        required_points: u64,
        consumed_points: u64,
        credit_points: u64,
    },
    #[error(
        "Insufficient fees to fund native verification requiring {required_points} points: {consumed_points} of \
         {allowance} allowance points already consumed"
    )]
    InsufficientFeesForNativeExecution {
        required_points: u64,
        consumed_points: u64,
        allowance: u64,
    },
    #[error(
        "Native verification points {consumed_points} exceed the per-transaction maximum {max_points}. Split the work \
         across transactions."
    )]
    MaxNativeExecutionPointsExceeded { consumed_points: u64, max_points: u64 },
    #[error("No fees paid from stealth transfer: {details}")]
    NoFeesPaid { details: String },
    #[error("No fee checkpoint")]
    NoFeeCheckpoint,

    #[error("Failed to load template '{address}': {details}")]
    FailedToLoadTemplate { address: TemplateAddress, details: String },
    #[error("Transaction Receipt already exists {address}")]
    TransactionReceiptAlreadyExists { address: TransactionReceiptAddress },
    #[error("Transaction Receipt not found")]
    TransactionReceiptNotFound,
    #[error("Component already exists {address}")]
    ComponentAlreadyExists { address: ComponentAddress },
    #[error("Cross-template call function error of function '{function}' on template '{template_address}': {details}")]
    CrossTemplateCallFunctionError {
        template_address: TemplateAddress,
        function: String,
        details: String,
    },
    #[error("Cross-template call failed for method '{method}' on component '{component_address}': {details}")]
    CrossTemplateCallMethodError {
        component_address: ComponentAddress,
        method: String,
        details: String,
    },
    #[error("Virtual substate not found: {address}")]
    VirtualSubstateNotFound { address: VirtualSubstateId },
    #[error("Invalid return value: {0}")]
    InvalidReturnValue(IndexedValueError),
    #[error("Attempt to pop auth scope stack but it was empty")]
    AuthScopeStackEmpty,
    #[error("Invalid deposit of bucket {bucket_id} has locked value amounting to {locked_amount}")]
    InvalidOpDepositLockedBucket { bucket_id: BucketId, locked_amount: Amount },
    #[error("Duplicate substate {address}")]
    DuplicateSubstate { address: SubstateId },
    #[error("Substate {id} is orphaned")]
    OrphanedSubstate { id: SubstateId },
    #[error("{} orphaned substate(s) detected: {}", .substates.len(), .substates.join(", "))]
    OrphanedSubstates { substates: Vec<String> },
    #[error("Attempted to finalise state but {remaining} call frame(s) remain on the stack")]
    CallFrameRemainingOnStack { remaining: usize },
    #[error("Duplicate reference to substate {address}")]
    DuplicateReference { address: SubstateId },
    #[error("Invalid argument: {0}")]
    ArgumentValidationError(#[from] ArgumentValidationError),
    #[error("Fee payment found in main intent, which is not allowed")]
    FeePaymentInMainIntent,

    #[error("BUG: [{function}] Invariant error {details}")]
    InvariantError { function: &'static str, details: String },
    #[error("Lock error: {0}")]
    LockError(#[from] LockError),
    #[error("{count} substate locks were still active after call")]
    DanglingSubstateLocks { count: usize },
    #[error("No active call frame")]
    NoActiveCallFrame,
    #[error("Max call depth {max_depth} exceeded")]
    MaxCallDepthExceeded { max_depth: usize },
    #[error("{action} can only be called from within a component context")]
    NotInComponentContext { action: ActionIdent },
    #[error("Engine call '{op}' is only permitted from within a template function invocation")]
    EngineCallOutsideInvocation { op: EngineOp },
    #[error("Duplicate bucket {bucket_id}")]
    DuplicateBucket { bucket_id: BucketId },
    #[error("Duplicate proof {proof_id}")]
    DuplicateProof { proof_id: ProofId },

    #[error("Address allocation not found with id {id}")]
    AddressAllocationNotFound { id: u32 },
    #[error("Address allocation with id {id} was not used")]
    AddressAllocationNotUsed { id: u32 },
    #[error("Address allocation type mismatch: got {id}, expected: {expected}")]
    AddressAllocationTypeMismatch { id: SubstateId, expected: &'static str },
    #[error("Allocated address does not have an associated template")]
    AddressAllocationNoTemplate,

    #[error("Invalid event topic '{topic}': {reason}")]
    InvalidEventTopic { topic: String, reason: String },

    #[error("Numeric conversion error: {details}")]
    NumericConversionError { details: String },
    #[error("Auth callback MUST return null, but it returned non-null")]
    UnexpectedNonNullInAuthHookReturn,

    #[error("Not supported: {details}")]
    NotSupported { details: String },

    #[error("Assert error: {0}")]
    AssertError(#[from] AssertError),

    #[error("Limit error: {0}")]
    LimitError(#[from] LimitError),

    #[error("Cross-template calls are not allowed in this context: {action:?}")]
    CrossTemplateCallNotAllowed { action: CallAction },
}

impl RuntimeError {
    pub fn to_reject_reason(&self, instruction_idx: Option<usize>) -> RejectReason {
        let instruction_prefix = if let Some(ref idx) = instruction_idx {
            format_args!("At instruction #{}: ", *idx)
        } else {
            format_args!("")
        };
        match self {
            Self::SubstateNotFound { id } => {
                RejectReason::SubstateNotFound(format!("{instruction_prefix}{id} not found",))
            },
            Self::RootSubstateNotFound { id } => RejectReason::SubstateNotFound(format!(
                "{instruction_prefix}Template referenced root substate but it was not found: {id}"
            )),
            Self::ReferencedSubstateNotFound { id } => RejectReason::SubstateNotFound(format!(
                "{instruction_prefix}Template referenced substate but it was not found: {id}"
            )),
            Self::InsufficientFeesPaid {
                fees_paid,
                required_fee,
            } => RejectReason::InsufficientFeesPaid(format!(
                "{instruction_prefix}Insufficient fees paid: {fees_paid}, required fees: {required_fee}"
            )),
            Self::FeePaymentInMainIntent => RejectReason::FeePaymentInMainIntent,
            err => RejectReason::ExecutionFailure(format!("{instruction_prefix}{err}")),
        }
    }
}

impl IsNotFoundError for RuntimeError {
    fn is_not_found_error(&self) -> bool {
        matches!(
            self,
            RuntimeError::SubstateNotFound { .. } |
                RuntimeError::ComponentNotFound { .. } |
                RuntimeError::VaultNotFound { .. } |
                RuntimeError::BucketNotFound { .. } |
                RuntimeError::ResourceNotFound { .. } |
                RuntimeError::NonFungibleNotFound { .. } |
                RuntimeError::ProofNotFound { .. } |
                RuntimeError::VirtualSubstateNotFound { .. }
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AssertError {
    #[error("The workspace value at {key} is not a bucket")]
    NotABucket { key: WorkspaceOffsetId },
    #[error("Assertion expected bucket to have resource {expected} but has {got}")]
    InvalidResource {
        expected: ResourceAddress,
        got: ResourceAddress,
    },
    #[error("Assertion expected bucket to have resource type {expected} but has {got}")]
    InvalidResourceType { expected: ResourceType, got: ResourceType },
    #[error("Assertion failed: expected bucket amount {got} to be {check} {expected}")]
    BucketAmountAssertionFail {
        expected: Amount,
        check: CheckOrd,
        got: Amount,
    },
    #[error("Assertion failed: expected bucket to contain {check} {nft}")]
    BucketContainsNonFungiblesAssertionFail { nft: NonFungibleId, check: NftCheck },
    #[error("Assertion failed: expected bucket to contain {check} the non-fungibles")]
    BucketContainsNonFungiblesAnyAssertionFail { check: NftCheck },
    #[error("Value is null but expected non-null")]
    ValueIsNull,
}

#[derive(Debug, thiserror::Error)]
pub enum TransactionCommitError {
    #[error("{count} dangling buckets remain after transaction execution")]
    DanglingBuckets { count: usize },
    #[error(
        "{count} dangling proofs remain after transaction execution. You may need to add a `DropAllProofsOnWorkspace` \
         instruction"
    )]
    DanglingProofs { count: usize },
    #[error("Locked value (amount: {locked_amount}) remaining in vault {vault_id}")]
    DanglingLockedValueInVault { vault_id: VaultId, locked_amount: Amount },
    #[error("{count} dangling address allocations remain after transaction execution")]
    DanglingAddressAllocations { count: usize },
    #[error("{count} dangling items in workspace after transaction execution")]
    WorkspaceNotEmpty { count: usize },
    #[error(transparent)]
    StateStoreError(#[from] StateStoreError),
    #[error(transparent)]
    IdProviderError(#[from] IdProviderError),
}

#[derive(Debug, thiserror::Error)]
pub enum ArgumentValidationError {
    #[error("Maximum number of stealth outputs exceeded: max {max_outputs}, got {actual_outputs}")]
    MaxStealthOutputsExceeded { max_outputs: usize, actual_outputs: usize },
    #[error("Maximum number of stealth inputs exceeded: max {max_inputs}, got {actual_inputs}")]
    MaxStealthInputsExceeded { max_inputs: usize, actual_inputs: usize },
    #[error("Maximum number of stealth transfers per transaction exceeded: max {max_transfers}")]
    MaxStealthTransfersPerTransactionExceeded { max_transfers: usize },
    #[error(
        "Maximum number of stealth transfers in the fee intent exceeded: max {max_transfers}. Perform further \
         transfers in the main intent."
    )]
    MaxFeeIntentStealthTransfersExceeded { max_transfers: usize },
    #[error("Maximum total stealth outputs per transaction exceeded: max {max_outputs}, got at least {actual_outputs}")]
    MaxStealthOutputsPerTransactionExceeded { max_outputs: usize, actual_outputs: usize },
    #[error("Maximum total stealth inputs per transaction exceeded: max {max_inputs}, got at least {actual_inputs}")]
    MaxStealthInputsPerTransactionExceeded { max_inputs: usize, actual_inputs: usize },
    #[error("Maximum number of confidential inputs exceeded: max {max_inputs}, got {actual_inputs}")]
    MaxConfidentialInputsExceeded { max_inputs: usize, actual_inputs: usize },
    #[error("Maximum number of confidential withdraws per transaction exceeded: max {max_withdraws}")]
    MaxConfidentialWithdrawsPerTransactionExceeded { max_withdraws: usize },
    #[error(
        "Maximum total confidential inputs per transaction exceeded: max {max_inputs}, got at least {actual_inputs}"
    )]
    MaxConfidentialInputsPerTransactionExceeded { max_inputs: usize, actual_inputs: usize },
    #[error("Too many arguments provided. Got {got}, max is {max}")]
    TooManyArguments { got: usize, max: usize },
    #[error("Blob index {index} out of bounds (transaction has {count} blob(s))")]
    BlobIndexOutOfBounds { index: u8, count: usize },
}

#[derive(Debug, thiserror::Error)]
pub enum LimitError {
    #[error("Substate size of {size} bytes exceeds the maximum allowed size of {} bytes", limits::ENGINE_LIMITS.max_substate_size)]
    SubstateSizeExceeded { size: usize },
    #[error("Log entry of {size} bytes exceeds maximum size of {} bytes", limits::ENGINE_LIMITS.max_log_size_bytes)]
    LogSizeExceeded { size: usize },
    #[error("Exceeded maximum number of logs per transaction: {}", limits::ENGINE_LIMITS.max_logs)]
    MaxLogsExceeded,
    #[error("Exceeded maximum number of events per transaction: {}", limits::ENGINE_LIMITS.max_events)]
    MaxEventsExceeded,
    #[error("Requested random bytes length {len} exceeds maximum of {} bytes", limits::ENGINE_LIMITS.max_random_bytes_len)]
    MaxRandomBytesLenExceeded { len: usize },
}
