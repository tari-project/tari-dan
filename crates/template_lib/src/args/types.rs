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
use minicbor::{CborLen, Decode, Encode};
use tari_template_abi::rust::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    fmt::{Display, Formatter},
    prelude::*,
};
use tari_template_lib_types::{
    AuthHook,
    ComponentAddress,
    LogLevel,
    Metadata,
    NonFungibleAddress,
    NonFungibleId,
    OwnerRule,
    ResourceAddress,
    ResourceType,
    UtxoId,
    VaultId,
    access_rules::{AccessRule, ComponentAccessRules, ResourceAccessRules, ResourceAuthAction},
    bytes::Bytes,
    confidential::{ConfidentialOutputStatement, ConfidentialWithdrawProof},
    crypto::CommitmentValueProof,
    stealth::StealthTransferStatement,
};

use crate::{
    args::freeze_flags::VaultFreezeFlags,
    models::{AddressAllocationId, BucketId, ComponentAddressAllocation, ProofId, ResourceAddressAllocation, VaultRef},
    template::BuiltinTemplate,
    types::{
        Amount,
        TemplateAddress,
        crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes},
    },
};
// -------------------------------- LOGS -------------------------------- //

/// Data needed for log emission from templates
#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EmitLogArg {
    #[n(0)]
    pub message: String,
    #[n(1)]
    pub level: LogLevel,
}

// -------------------------------- Component -------------------------------- //

/// An operation over a component
#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ComponentInvokeArg {
    #[n(0)]
    pub component_ref: ComponentRef,
    #[n(1)]
    pub action: ComponentAction,
    #[n(2)]
    pub args: Vec<Bytes>,
}

/// The possible actions that can be performed on components
#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ComponentAction {
    #[n(0)]
    Create,
    #[n(1)]
    GetState,
    #[n(2)]
    SetState,
    #[n(3)]
    SetAccessRules,
    #[n(4)]
    GetTemplateAddress,
    #[n(5)]
    GetOwnerProof,
}

/// Encapsulates all the ways that a component can be referenced
#[derive(Clone, Copy, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ComponentRef {
    #[n(0)]
    Component,
    #[n(1)]
    Ref(#[n(0)] ComponentAddress),
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
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ComponentRef::Component => write!(f, "Component"),
            ComponentRef::Ref(addr) => write!(f, "Ref({})", addr),
        }
    }
}

/// A component creation operation
#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CreateComponentArg {
    #[n(0)]
    pub encoded_state: tari_bor::Value,
    #[n(1)]
    pub owner_rule: OwnerRule,
    #[n(2)]
    pub access_rules: ComponentAccessRules,
    #[n(3)]
    pub address_allocation: Option<ComponentAddressAllocation>,
}

// -------------------------------- Events -------------------------------- //

/// An event emission operation
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EmitEventArg {
    #[n(0)]
    pub topic: String,
    #[n(1)]
    pub payload: Metadata,
}

// -------------------------------- Resource -------------------------------- //

/// An operation over a resource
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResourceInvokeArg {
    #[n(0)]
    pub resource_ref: ResourceRef,
    #[n(1)]
    pub action: ResourceAction,
    #[n(2)]
    pub args: Vec<Bytes>,
}

/// Encapsulates all the ways that a resource can be referenced
#[derive(Clone, Copy, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ResourceRef {
    #[n(0)]
    Resource,
    #[n(1)]
    Ref(#[n(0)] ResourceAddress),
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
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ResourceRef::Resource => write!(f, "Resource"),
            ResourceRef::Ref(addr) => write!(f, "Ref({})", addr),
        }
    }
}

/// An action performed on a Resource
#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ResourceAction {
    /// Create a new resource
    #[n(0)]
    Create,
    /// Mint more of a resource
    #[n(1)]
    Mint,
    /// Recall an amount resource from a vault
    #[n(2)]
    Recall,
    /// Update on a non-fungible
    #[n(3)]
    UpdateNonFungibleData,
    /// Get the total supply of a resource
    #[n(4)]
    GetTotalSupply,
    /// Get the [ResourceInfo](tari_template_lib_types::ResourceType) of a resource
    #[n(5)]
    GetResourceInfo,
    /// Gets a non-fungible resource by its ID
    #[n(6)]
    GetNonFungible,
    /// Update a single access rule of a resource. Authorization is gated by the field's
    /// [`UpdateRule`](tari_template_lib_types::access_rules::UpdateRule).
    #[n(7)]
    UpdateAccessRule,
    /// Sets the freeze flags on a vault of a resource.
    #[n(8)]
    SetVaultFreeze,
    /// Executes a stealth transfer for the resource
    #[n(9)]
    StealthTransfer,
    /// Un/freezes one or more stealth UTXOs of a resource
    #[n(10)]
    SetStealthUtxosFreeze,
    /// Burns a stealth UTXO of a resource
    #[n(11)]
    StealthUtxoBurn,
    /// Update the metadata of a resource (token symbol remains immutable once set).
    #[n(12)]
    UpdateMetadata,
    /// Un/freezes one or more confidential outputs of a resource
    #[n(13)]
    SetConfidentialOutputsFreeze,
    /// Replace or remove a resource's authorization hook. Authorization is gated by
    /// [`ResourceAccessRules::auth_hook_updater`](tari_template_lib_types::access_rules::ResourceAccessRules::auth_hook_updater).
    #[n(14)]
    UpdateAuthHook,
}

/// All the possible minting operation types
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MintArg {
    #[n(0)]
    Fungible {
        #[n(0)]
        amount: Amount,
    },
    #[n(1)]
    NonFungible {
        #[n(0)]
        tokens: BTreeMap<NonFungibleId, (tari_bor::Value, tari_bor::Value)>,
    },
    #[n(2)]
    Confidential {
        #[n(0)]
        statement: Box<ConfidentialOutputStatement>,
        /// Proves the value of each commitment the statement mints, keyed by commitment. One is required per
        /// commitment when the resource tracks total supply, since the engine cannot otherwise learn the minted
        /// value. Entries for commitments the statement does not mint are ignored.
        #[n(1)]
        value_proofs: BTreeMap<PedersenCommitmentBytes, CommitmentValueProof>,
    },
    #[n(3)]
    Stealth {
        #[n(0)]
        amount: Amount,
    },
}

impl MintArg {
    pub fn as_resource_type(&self) -> ResourceType {
        match self {
            MintArg::Fungible { .. } => ResourceType::Fungible,
            MintArg::NonFungible { .. } => ResourceType::NonFungible,
            MintArg::Confidential { .. } => ResourceType::Confidential,
            MintArg::Stealth { .. } => ResourceType::Stealth,
        }
    }
}

/// A resource creation operation
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CreateResourceArg {
    #[n(0)]
    pub resource_type: ResourceType,
    #[n(1)]
    pub owner_rule: OwnerRule,
    #[n(2)]
    pub access_rules: ResourceAccessRules,
    #[n(3)]
    pub metadata: Metadata,
    #[n(4)]
    pub mint_arg: Option<MintArg>,
    #[n(5)]
    pub view_key: Option<RistrettoPublicKeyBytes>,
    #[n(6)]
    pub authorize_hook: Option<AuthHook>,
    #[n(7)]
    pub address_allocation: Option<ResourceAddressAllocation>,
    #[n(8)]
    pub divisibility: u8,
    #[n(9)]
    pub is_total_supply_tracking_enabled: bool,
}

/// A resource minting operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MintResourceArg {
    #[n(0)]
    pub mint_arg: MintArg,
}

/// Stealth transfer operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StealthTransferResourceArg {
    #[n(0)]
    pub transfer: StealthTransferStatement,
    #[n(1)]
    pub input_bucket: Option<BucketId>,
}

/// A resource minting operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResourceGetNonFungibleArg {
    #[n(0)]
    pub id: NonFungibleId,
}

/// A non-fungible resource update operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResourceUpdateNonFungibleDataArg {
    #[n(0)]
    pub id: NonFungibleId,
    #[n(1)]
    pub data: tari_bor::Value,
}

/// An argument used to update a single field of a resource's [`ResourceAccessRules`].
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct UpdateAccessRuleArg {
    #[n(0)]
    pub action: ResourceAuthAction,
    #[n(1)]
    pub new_rule: AccessRule,
}

/// An argument used to replace or remove a resource's authorization hook.
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct UpdateAuthHookArg {
    /// The hook to install, or `None` to remove the resource's hook entirely.
    #[n(0)]
    pub auth_hook: Option<AuthHook>,
}

/// A convenience enum that allows to specify resource types
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ResourceDiscriminator {
    /// Select all tokens
    #[n(0)]
    Everything,
    /// Select a specific amount of fungible (public or stealth) tokens
    #[n(1)]
    Fungible {
        #[n(0)]
        amount: Amount,
    },
    /// Select specific non-fungible tokens
    #[n(2)]
    NonFungible {
        #[n(0)]
        tokens: BTreeSet<NonFungibleId>,
    },
    /// Select specific confidential commitments and a revealed amount
    #[n(3)]
    Confidential {
        #[n(0)]
        commitments: BTreeSet<PedersenCommitmentBytes>,
        #[n(1)]
        revealed_amount: Amount,
    },
}

impl Display for ResourceDiscriminator {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ResourceDiscriminator::Everything => write!(f, "Everything"),
            ResourceDiscriminator::Fungible { amount } => write!(f, "Fungible({})", amount),
            ResourceDiscriminator::NonFungible { tokens } => write!(f, "NonFungible({} token(s))", tokens.len()),
            ResourceDiscriminator::Confidential {
                commitments,
                revealed_amount,
            } => {
                write!(
                    f,
                    "Confidential({} commitment(s), {})",
                    commitments.len(),
                    revealed_amount
                )
            },
        }
    }
}

/// A resource recall operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RecallResourceArg {
    #[n(0)]
    pub vault_id: VaultId,
    #[n(1)]
    pub resource: ResourceDiscriminator,
}

/// A resource freeze operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FreezeResourceArg {
    #[n(0)]
    pub vault_id: VaultId,
    #[n(1)]
    pub flags: VaultFreezeFlags,
}

// -------------------------------- Vault -------------------------------- //

/// A vault operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VaultInvokeArg {
    #[n(0)]
    pub vault_ref: VaultRef,
    #[n(1)]
    pub action: VaultAction,
    #[n(2)]
    pub args: Vec<Bytes>,
}

/// The possible actions that can be performed on vaults
#[derive(Debug, Clone, Copy, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VaultAction {
    #[n(0)]
    Create,
    #[n(1)]
    Deposit,
    #[n(2)]
    Withdraw,
    #[n(3)]
    GetBalance,
    #[n(4)]
    GetLockedBalance,
    #[n(5)]
    GetResourceAddress,
    #[n(6)]
    GetNonFungibleIds,
    #[n(7)]
    GetCommitmentCount,
    #[n(8)]
    PayFee,
    #[n(9)]
    CreateProofByResource,
    #[n(10)]
    CreateProofByFungibleAmount,
    #[n(11)]
    CreateProofByNonFungibles,
    #[n(12)]
    CreateProofByConfidentialResource,
    #[n(13)]
    GetNonFungibles,
}

impl VaultAction {
    pub fn requires_write_access(&self) -> bool {
        use VaultAction::*;
        !matches!(
            self,
            GetBalance |
                GetLockedBalance |
                GetResourceAddress |
                GetNonFungibleIds |
                GetCommitmentCount |
                GetNonFungibles
        )
    }
}

/// A vault withdraw operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VaultWithdrawArg {
    #[n(0)]
    Fungible {
        #[n(0)]
        amount: Amount,
    },
    #[n(1)]
    NonFungible {
        #[n(0)]
        ids: BTreeSet<NonFungibleId>,
    },
    #[n(2)]
    Confidential {
        #[n(0)]
        proof: Box<ConfidentialWithdrawProof>,
    },
    #[n(3)]
    Stealth {
        #[n(0)]
        amount: Amount,
    },
}

// -------------------------------- Fees -------------------------------- //

/// A fee payment operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PayFeeArg {
    #[n(0)]
    pub amount: Amount,
    #[n(1)]
    pub statement: Option<StealthTransferStatement>,
}

// -------------------------------- Bucket -------------------------------- //

/// A bucket operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BucketInvokeArg {
    #[n(0)]
    pub bucket_ref: BucketRef,
    #[n(1)]
    pub action: BucketAction,
    #[n(2)]
    pub args: Vec<Bytes>,
}

/// Encapsulates all the ways that a bucket can be referenced
#[derive(Clone, Copy, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BucketRef {
    #[n(0)]
    Bucket(#[n(0)] ResourceAddress),
    #[n(1)]
    Ref(#[n(0)] BucketId),
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
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            BucketRef::Bucket(addr) => write!(f, "Bucket({})", addr),
            BucketRef::Ref(id) => write!(f, "Ref({})", id),
        }
    }
}

/// The possible actions that can be performed on buckets
#[derive(Clone, Copy, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BucketAction {
    #[n(0)]
    GetResourceAddress,
    #[n(1)]
    GetResourceType,
    #[n(2)]
    GetAmount,
    #[n(3)]
    Take,
    #[n(4)]
    TakeConfidential,
    #[n(5)]
    Join,
    #[n(6)]
    Burn,
    #[n(7)]
    CreateProof,
    #[n(8)]
    GetNonFungibleIds,
    #[n(9)]
    GetNonFungibles,
    #[n(10)]
    CountConfidentialCommitments,
    #[n(11)]
    DropEmpty,
}

/// A bucket burn operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BucketBurnArg {
    #[n(0)]
    pub bucket_id: BucketId,
}

/// BucketAction::GetAmount argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BucketGetAmountArg {
    #[n(0)]
    AmountOnly,
    #[n(1)]
    LockedOnly,
    #[n(2)]
    AmountAndLocked,
    #[n(3)]
    Everything,
}

// -------------------------------- Workspace -------------------------------- //

/// The possible actions that can be performed on workspace variables
#[derive(Clone, Copy, Debug)]
pub enum WorkspaceAction {
    PutLastInstructionOutput,
    Get,
    DropAllProofs,
    Assert,
    DropAll,
}

// -------------------------------- NonFungible -------------------------------- //

/// A non-fungible operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NonFungibleInvokeArg {
    #[n(0)]
    pub address: NonFungibleAddress,
    #[n(1)]
    pub action: NonFungibleAction,
    #[n(2)]
    pub args: Vec<Bytes>,
}

/// The possible actions that can be performed on non-fungible resources
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum NonFungibleAction {
    #[n(0)]
    GetData,
    #[n(1)]
    GetMutableData,
}

// -------------------------------- Consensus -------------------------------- //

/// A consensus operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ConsensusInvokeArg {
    #[n(0)]
    pub action: ConsensusAction,
}

/// The possible actions that can be performed related to consensus
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ConsensusAction {
    #[n(0)]
    GetCurrentEpoch,
    #[n(1)]
    GetCurrentEpochHash,
}

// -------------------------------- GenerateRandom -------------------------------- //

/// A random generation operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GenerateRandomInvokeArg {
    #[n(0)]
    pub action: GenerateRandomAction,
}

/// The possible actions that can be performed related to random generation
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum GenerateRandomAction {
    #[n(0)]
    GetRandomBytes {
        #[n(0)]
        len: u32,
    },
}

// -------------------------------- CallerContext -------------------------------- //

/// The possible actions that can be performed related to the caller context
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CallerContextAction {
    #[n(0)]
    GetCallerPublicKey,
    #[n(1)]
    GetComponentAddress,
    #[n(2)]
    GetSignerProof,
}

/// A caller context operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CallerContextInvokeArg {
    #[n(0)]
    pub action: CallerContextAction,
    #[n(1)]
    pub args: Vec<Bytes>,
}

// -------------------------------- AddressAllocation -------------------------------- //

/// The possible actions that can be performed related to the caller context
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AddressAllocationInvokeArg {
    #[n(0)]
    GetAddress(#[n(0)] AddressAllocationId),
    #[n(1)]
    CreateComponentAllocation {
        #[n(0)]
        public_key: Option<RistrettoPublicKeyBytes>,
    },
    #[n(2)]
    CreateResourceAllocation,
}

/// Result af an address allocation based on the input substate type
#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AllocateAddressResult {
    #[n(0)]
    ComponentAddress(#[n(0)] ComponentAddressAllocation),
    #[n(1)]
    ResourceAddress(#[n(0)] ResourceAddressAllocation),
}

// -------------------------------- CallInvoke -------------------------------- //

/// A call operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CallInvokeArg {
    #[n(0)]
    pub action: CallAction,
    #[n(1)]
    pub args: Vec<Bytes>,
}

/// All the possible call operation types
#[derive(Debug, Clone, Copy, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CallAction {
    /// Call to a template's function
    #[n(0)]
    CallFunction,
    /// Call to a component's method
    #[n(1)]
    CallMethod,
}

/// A template's function call operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CallFunctionArg {
    #[n(0)]
    pub template_address: TemplateAddress,
    #[n(1)]
    pub function: String,
    #[n(2)]
    pub args: Vec<Bytes>,
}

/// A component's method call operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CallMethodArg {
    #[n(0)]
    pub component_address: ComponentAddress,
    #[n(1)]
    pub method: String,
    #[n(2)]
    pub args: Vec<Bytes>,
}

// -------------------------------- ProofInvoke -------------------------------- //

/// A proof-related operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProofInvokeArg {
    #[n(0)]
    pub proof_ref: ProofRef,
    #[n(1)]
    pub action: ProofAction,
    #[n(2)]
    pub args: Vec<Bytes>,
}

/// All the possible ways to reference a proof
#[derive(Clone, Copy, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ProofRef {
    #[n(0)]
    Proof(#[n(0)] ResourceAddress),
    #[n(1)]
    Ref(#[n(0)] ProofId),
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
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ProofRef::Proof(addr) => write!(f, "Proof({})", addr),
            ProofRef::Ref(id) => write!(f, "Ref({})", id),
        }
    }
}

/// All the possible actions that can be performed on proofs
#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ProofAction {
    #[n(0)]
    GetAmount,
    #[n(1)]
    GetResourceAddress,
    #[n(2)]
    GetResourceType,
    #[n(3)]
    GetNonFungibles,
    #[n(4)]
    Authorize,
    #[n(5)]
    DropAuthorize,
    #[n(6)]
    Drop,
}

/// An argument to represent a proof of a vault's fungible amount
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VaultCreateProofByFungibleAmountArg {
    #[n(0)]
    pub amount: Amount,
}

/// An argument to represent a proof of a vault's non-fungible presence
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VaultCreateProofByNonFungiblesArg {
    #[n(0)]
    pub ids: BTreeSet<NonFungibleId>,
}

/// TODO: confidential. Zero knowledge proof of commitment factors
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CreateProofOfResourceByConfidentialArg {
    // pub proof: ConfidentialProofOfKnowledge
}

// -------------------------------- BuiltinTemplate -------------------------------- //

/// A template builtin operation argument
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BuiltinTemplateInvokeArg {
    #[n(0)]
    pub action: BuiltinTemplateAction,
}

/// The possible actions that can be performed related to builtin templates
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BuiltinTemplateAction {
    #[n(0)]
    GetTemplateAddress {
        #[n(0)]
        builtin: BuiltinTemplate,
    },
}

// -------------------------------- UTXOs -------------------------------- //

#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SetFreezeStealthUtxosArg {
    #[n(0)]
    pub utxos: Vec<UtxoId>,
    #[n(1)]
    pub freeze: bool,
}

#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SetFreezeConfidentialOutputsArg {
    #[n(0)]
    pub commitments: Vec<PedersenCommitmentBytes>,
    #[n(1)]
    pub freeze: bool,
}

#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BurnStealthUtxoArg {
    #[n(0)]
    pub utxo_id: UtxoId,
    #[n(1)]
    pub value_proof: Option<CommitmentValueProof>,
}

/// Arguments for [`BucketAction::Burn`].
#[derive(Clone, Debug, Default, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BurnBucketArg {
    /// Proves the value of each confidential commitment the bucket holds, keyed by commitment. One is required per
    /// commitment when the resource tracks total supply, since the engine cannot otherwise learn how much value the
    /// burn destroys. Entries for commitments the bucket does not hold are ignored.
    #[n(0)]
    pub value_proofs: BTreeMap<PedersenCommitmentBytes, CommitmentValueProof>,
}

// -------------------------------- SpendContext -------------------------------- //

/// The read-only introspection actions a spend script may perform over the spending transfer. The scope is the current
/// `StealthTransferStatement` only. Confidential values are never exposed — only commitments and
/// `minimum_value_promise`.
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SpendContextAction {
    /// The stealth inputs being spent (commitments).
    #[n(0)]
    Inputs,
    /// The stealth outputs being created (commitment, minimum value promise, spend condition, tag).
    #[n(1)]
    Outputs,
    /// The index, commitment and committed `condition_root` of the input whose condition is executing.
    #[n(2)]
    CurrentInput,
    // `#[n(3)]` was `InvokingCondition`; the invoking condition tree is now exposed via `CurrentInput`'s
    // `condition_root`. The index is left unused rather than renumbered.
    /// The total revealed amount being spent by the transfer.
    #[n(4)]
    RevealedInputAmount,
    /// The total revealed amount being output by the transfer.
    #[n(5)]
    RevealedOutputAmount,
    /// Verifies the covenant sub-balance proof for the invoking partition, permitting at most `max_revealed` cleartext
    /// to leave it. Returns whether the partition's value is conserved within that allowance.
    #[n(6)]
    AssertCovenantBalanced {
        #[n(0)]
        max_revealed: u64,
    },
    /// The raw spender-supplied witness `data` blob for the invoking input (empty if none was provided).
    #[n(7)]
    WitnessData,
}

/// A spend-context introspection operation argument.
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SpendContextInvokeArg {
    #[n(0)]
    pub action: SpendContextAction,
}
