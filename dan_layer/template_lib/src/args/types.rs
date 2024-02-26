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

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use tari_template_abi::rust::{
    fmt::{Display, Formatter},
    str::FromStr,
};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{
    args::Arg,
    auth::{OwnerRule, ResourceAccessRules},
    crypto::PedersonCommitmentBytes,
    models::{
        AddressAllocation,
        Amount,
        BucketId,
        ComponentAddress,
        ConfidentialWithdrawProof,
        Metadata,
        NonFungibleAddress,
        NonFungibleId,
        ProofId,
        ResourceAddress,
        VaultId,
        VaultRef,
    },
    prelude::{ComponentAccessRules, ConfidentialOutputProof, TemplateAddress},
    resource::ResourceType,
    template::BuiltinTemplate,
    Hash,
};

// -------------------------------- LOGS -------------------------------- //

/// Data needed for log emission from templates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmitLogArg {
    pub message: String,
    pub level: LogLevel,
}

/// All the possible log levels
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Error => write!(f, "ERROR"),
            LogLevel::Warn => write!(f, "WARN"),
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Debug => write!(f, "DEBUG"),
        }
    }
}

impl FromStr for LogLevel {
    type Err = LogLevelParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ERROR" => Ok(LogLevel::Error),
            "WARN" => Ok(LogLevel::Warn),
            "INFO" => Ok(LogLevel::Info),
            "DEBUG" => Ok(LogLevel::Debug),
            _ => Err(LogLevelParseError(s.to_string())),
        }
    }
}

/// Error when trying to parse a log level from an `String`
#[derive(Debug)]
pub struct LogLevelParseError(String);

impl Display for LogLevelParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Failed to parse log level '{}'", self.0)
    }
}

impl std::error::Error for LogLevelParseError {}

// -------------------------------- Component -------------------------------- //

/// An operation over a component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentInvokeArg {
    pub component_ref: ComponentRef,
    pub action: ComponentAction,
    pub args: Vec<Vec<u8>>,
}

/// The possible actions that can be performed on components
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComponentAction {
    Create,
    GetState,
    SetState,
    SetAccessRules,
    GetTemplateAddress,
}

/// Encapsulates all the ways that a component can be referenced
#[derive(Clone, Copy, Hash, Debug, Serialize, Deserialize)]
pub enum ComponentRef {
    Component,
    Ref(ComponentAddress),
}

impl ComponentRef {
    pub fn as_component_address(&self) -> Option<ComponentAddress> {
        match self {
            ComponentRef::Component => None,
            ComponentRef::Ref(addr) => Some(*addr),
        }
    }
}

impl Display for ComponentRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ComponentRef::Component => write!(f, "Component"),
            ComponentRef::Ref(addr) => write!(f, "Ref({})", addr),
        }
    }
}

/// A component creation operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateComponentArg {
    pub encoded_state: tari_bor::Value,
    pub owner_rule: OwnerRule,
    pub access_rules: ComponentAccessRules,
    pub component_id: Option<Hash>,
    pub address_allocation: Option<AddressAllocation<ComponentAddress>>,
}

// -------------------------------- Events -------------------------------- //

/// An event emission operation
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmitEventArg {
    pub topic: String,
    pub payload: Metadata,
}

// -------------------------------- Resource -------------------------------- //

/// An operation over a resource
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceInvokeArg {
    pub resource_ref: ResourceRef,
    pub action: ResourceAction,
    pub args: Vec<Vec<u8>>,
}

/// Encapsulates all the ways that a resource can be referenced
#[derive(Clone, Copy, Hash, Debug, Serialize, Deserialize)]
pub enum ResourceRef {
    Resource,
    Ref(ResourceAddress),
}

impl ResourceRef {
    pub fn as_resource_address(&self) -> Option<ResourceAddress> {
        match self {
            ResourceRef::Resource => None,
            ResourceRef::Ref(addr) => Some(*addr),
        }
    }
}

impl From<ResourceAddress> for ResourceRef {
    fn from(addr: ResourceAddress) -> Self {
        ResourceRef::Ref(addr)
    }
}

impl Display for ResourceRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceRef::Resource => write!(f, "Resource"),
            ResourceRef::Ref(addr) => write!(f, "Ref({})", addr),
        }
    }
}

/// The possible actions that can be performed on resources
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ResourceAction {
    Create,
    Mint,
    Recall,
    UpdateNonFungibleData,
    GetTotalSupply,
    GetResourceType,
    GetNonFungible,
    UpdateAccessRules,
}

/// All the possible minting operation types
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MintArg {
    Fungible {
        amount: Amount,
    },
    NonFungible {
        tokens: BTreeMap<NonFungibleId, (Vec<u8>, Vec<u8>)>,
    },
    Confidential {
        proof: Box<ConfidentialOutputProof>,
    },
}

/// A resource creation operation
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateResourceArg {
    pub resource_type: ResourceType,
    pub owner_rule: OwnerRule,
    pub access_rules: ResourceAccessRules,
    pub metadata: Metadata,
    pub mint_arg: Option<MintArg>,
}

/// A resource minting operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MintResourceArg {
    pub mint_arg: MintArg,
}

/// A resource minting operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceGetNonFungibleArg {
    pub id: NonFungibleId,
}

/// A non-fungible resource update operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceUpdateNonFungibleDataArg {
    pub id: NonFungibleId,
    pub data: Vec<u8>,
}

/// A convenience enum that allows to specify resource types
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ResourceDiscriminator {
    Everything,
    Fungible {
        amount: Amount,
    },
    NonFungible {
        tokens: BTreeSet<NonFungibleId>,
    },
    Confidential {
        commitments: BTreeSet<PedersonCommitmentBytes>,
        revealed_amount: Amount,
    },
}

/// A resource recall operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecallResourceArg {
    pub vault_id: VaultId,
    pub resource: ResourceDiscriminator,
}
// -------------------------------- Vault -------------------------------- //

/// A vault operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultInvokeArg {
    pub vault_ref: VaultRef,
    pub action: VaultAction,
    pub args: Vec<Vec<u8>>,
}

/// The possible actions that can be performed on vaults
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum VaultAction {
    Create,
    Deposit,
    Withdraw,
    GetBalance,
    GetResourceAddress,
    GetNonFungibleIds,
    GetCommitmentCount,
    ConfidentialReveal,
    PayFee,
    CreateProofByResource,
    CreateProofByFungibleAmount,
    CreateProofByNonFungibles,
    CreateProofByConfidentialResource,
    GetNonFungibles,
}

/// A vault withdraw operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum VaultWithdrawArg {
    Fungible { amount: Amount },
    NonFungible { ids: BTreeSet<NonFungibleId> },
    Confidential { proof: Box<ConfidentialWithdrawProof> },
}

// -------------------------------- Confidential -------------------------------- //

/// A confidential resource reveal operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfidentialRevealArg {
    pub proof: ConfidentialWithdrawProof,
}

// -------------------------------- Fees -------------------------------- //

/// A fee payment operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PayFeeArg {
    pub amount: Amount,
    pub proof: Option<ConfidentialWithdrawProof>,
}

// -------------------------------- Bucket -------------------------------- //

/// A bucket operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BucketInvokeArg {
    pub bucket_ref: BucketRef,
    pub action: BucketAction,
    pub args: Vec<Vec<u8>>,
}

/// Encapsulates all the ways that a bucket can be referenced
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum BucketRef {
    Bucket(ResourceAddress),
    Ref(BucketId),
}

impl BucketRef {
    pub fn resource_address(&self) -> Option<ResourceAddress> {
        match self {
            BucketRef::Bucket(addr) => Some(*addr),
            BucketRef::Ref(_) => None,
        }
    }

    pub fn bucket_id(&self) -> Option<BucketId> {
        match self {
            BucketRef::Bucket(_) => None,
            BucketRef::Ref(id) => Some(*id),
        }
    }
}

impl Display for BucketRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            BucketRef::Bucket(addr) => write!(f, "Bucket({})", addr),
            BucketRef::Ref(id) => write!(f, "Ref({})", id),
        }
    }
}

/// The possible actions that can be performed on buckets
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum BucketAction {
    GetResourceAddress,
    GetResourceType,
    GetAmount,
    Take,
    TakeConfidential,
    RevealConfidential,
    Burn,
    CreateProof,
    GetNonFungibleIds,
    GetNonFungibles,
    CountConfidentialCommitments,
}

/// A bucket burn operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BucketBurnArg {
    pub bucket_id: BucketId,
}

// -------------------------------- Workspace -------------------------------- //

/// The possible actions that can be performed on workspace variables
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum WorkspaceAction {
    PutLastInstructionOutput,
    Get,
    ListBuckets,
    DropAllProofs,
}

/// A workspace operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceInvokeArg {
    pub action: WorkspaceAction,
    pub args: Vec<Vec<u8>>,
}

// -------------------------------- NonFungible -------------------------------- //

/// A non-fungible operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NonFungibleInvokeArg {
    pub address: NonFungibleAddress,
    pub action: NonFungibleAction,
    pub args: Vec<Vec<u8>>,
}

/// The possible actions that can be performed on non-fungible resources
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NonFungibleAction {
    GetData,
    GetMutableData,
}

// -------------------------------- Consensus -------------------------------- //

/// A consensus operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsensusInvokeArg {
    pub action: ConsensusAction,
}

/// The possible actions that can be performed related to consensus
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ConsensusAction {
    GetCurrentEpoch,
}

// -------------------------------- GenerateRandom -------------------------------- //

/// A random generation operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenerateRandomInvokeArg {
    pub action: GenerateRandomAction,
}

/// The possible actions that can be performed related to random generation
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GenerateRandomAction {
    GetRandomBytes { len: u32 },
}

// -------------------------------- CallerContext -------------------------------- //

/// A caller context operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallerContextInvokeArg {
    pub action: CallerContextAction,
}

/// The possible actions that can be performed related to the caller context
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CallerContextAction {
    GetCallerPublicKey,
    GetComponentAddress,
    AllocateNewComponentAddress,
}

// -------------------------------- CallInvoke -------------------------------- //

/// A call operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallInvokeArg {
    pub action: CallAction,
    pub args: Vec<Vec<u8>>,
}

/// All the possible call operation types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CallAction {
    /// Call to a template's function
    CallFunction,
    /// Call to a component's method
    CallMethod,
}

/// A template's function call operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallFunctionArg {
    pub template_address: TemplateAddress,
    pub function: String,
    pub args: Vec<Arg>,
}

/// A component's method call operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallMethodArg {
    pub component_address: ComponentAddress,
    pub method: String,
    pub args: Vec<Arg>,
}

// -------------------------------- ProofInvoke -------------------------------- //

/// A proof-related operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProofInvokeArg {
    pub proof_ref: ProofRef,
    pub action: ProofAction,
    pub args: Vec<Vec<u8>>,
}

/// All the possible ways to reference a proof
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ProofRef {
    Proof(ResourceAddress),
    Ref(ProofId),
}

impl ProofRef {
    pub fn resource_address(&self) -> Option<ResourceAddress> {
        match self {
            ProofRef::Proof(addr) => Some(*addr),
            ProofRef::Ref(_) => None,
        }
    }

    pub fn proof_id(&self) -> Option<ProofId> {
        match self {
            ProofRef::Proof(_) => None,
            ProofRef::Ref(id) => Some(*id),
        }
    }
}

impl Display for ProofRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ProofRef::Proof(addr) => write!(f, "Proof({})", addr),
            ProofRef::Ref(id) => write!(f, "Ref({})", id),
        }
    }
}

/// All the possible actions that can be performed on proofs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProofAction {
    GetAmount,
    GetResourceAddress,
    GetResourceType,
    Authorize,
    DropAuthorize,
    Drop,
}

/// An argument to represent a proof of a vault's fungible amount
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultCreateProofByFungibleAmountArg {
    pub amount: Amount,
}

/// An argument to represent a proof of a vault's non-fungible presence
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultCreateProofByNonFungiblesArg {
    pub ids: BTreeSet<NonFungibleId>,
}

/// TODO: confidential. Zero knowledge proof of commitment factors
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateProofOfResourceByConfidentialArg {
    // pub proof: ConfidentialProofOfKnowledge
}

// -------------------------------- BuiltinTemplate -------------------------------- //

/// A template builtin operation argument
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuiltinTemplateInvokeArg {
    pub action: BuiltinTemplateAction,
}

/// The possible actions that can be performed related to builtin templates
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum BuiltinTemplateAction {
    GetTemplateAddress { bultin: BuiltinTemplate },
}
