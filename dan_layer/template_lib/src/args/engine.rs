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

use std::collections::BTreeSet;

use tari_bor::{borsh, Decode, Encode};
use tari_template_abi::rust::{
    collections::HashMap,
    fmt::{Display, Formatter},
    str::FromStr,
};

use crate::{
    models::{
        Amount,
        BucketId,
        ComponentAddress,
        Metadata,
        NonFungibleAddress,
        NonFungibleId,
        ResourceAddress,
        VaultRef,
    },
    prelude::{AccessRules, ConfidentialProof},
    resource::ResourceType,
};

// -------------------------------- LOGS -------------------------------- //
#[derive(Debug, Clone, Encode, Decode)]
pub struct EmitLogArg {
    pub message: String,
    pub level: LogLevel,
}

#[derive(Debug, Clone, Copy, Encode, Decode, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
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

#[derive(Debug)]
pub struct LogLevelParseError(String);

impl Display for LogLevelParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Failed to parse log level '{}'", self.0)
    }
}

impl std::error::Error for LogLevelParseError {}

// -------------------------------- Component -------------------------------- //
#[derive(Debug, Clone, Encode, Decode)]
pub struct ComponentInvokeArg {
    pub component_ref: ComponentRef,
    pub action: ComponentAction,
    pub args: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Decode, Encode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ComponentAction {
    Get,
    Create,
    SetState,
    SetAccessRules,
}

#[derive(Clone, Copy, Hash, Debug, Decode, Encode)]
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

#[derive(Debug, Clone, Decode, Encode)]
pub struct CreateComponentArg {
    pub module_name: String,
    pub encoded_state: Vec<u8>,
    pub access_rules: AccessRules,
}

// -------------------------------- Resource -------------------------------- //
#[derive(Clone, Debug, Decode, Encode)]
pub struct ResourceInvokeArg {
    pub resource_ref: ResourceRef,
    pub action: ResourceAction,
    pub args: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Hash, Debug, Decode, Encode)]
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Decode, Encode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ResourceAction {
    GetTotalSupply,
    GetResourceType,
    GetNonFungible,
    Create,
    Mint,
    UpdateNonFungibleData,
}

#[derive(Clone, Debug, Decode, Encode)]
pub enum MintArg {
    Fungible {
        amount: Amount,
    },
    NonFungible {
        tokens: HashMap<NonFungibleId, (Vec<u8>, Vec<u8>)>,
    },
    Confidential {
        proof: ConfidentialProof,
    },
}

#[derive(Clone, Debug, Decode, Encode)]
pub struct CreateResourceArg {
    pub resource_type: ResourceType,
    pub metadata: Metadata,
    pub mint_arg: Option<MintArg>,
}

#[derive(Clone, Debug, Decode, Encode)]
pub struct MintResourceArg {
    pub mint_arg: MintArg,
}

#[derive(Clone, Debug, Decode, Encode)]
pub struct ResourceGetNonFungibleArg {
    pub id: NonFungibleId,
}

#[derive(Clone, Debug, Decode, Encode)]
pub struct ResourceUpdateNonFungibleDataArg {
    pub id: NonFungibleId,
    pub data: Vec<u8>,
}

// -------------------------------- Vault -------------------------------- //
#[derive(Clone, Debug, Decode, Encode)]
pub struct VaultInvokeArg {
    pub vault_ref: VaultRef,
    pub action: VaultAction,
    pub args: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Decode, Encode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VaultAction {
    Create,
    Deposit,
    Withdraw,
    WithdrawAll,
    GetBalance,
    GetResourceAddress,
    GetNonFungibleIds,
}

#[derive(Clone, Debug, Decode, Encode)]
pub enum VaultWithdrawArg {
    Fungible { amount: Amount },
    NonFungible { ids: BTreeSet<NonFungibleId> },
    Confidential { proofs: Vec<ConfidentialProof> },
}

// -------------------------------- Bucket -------------------------------- //
#[derive(Clone, Debug, Decode, Encode)]
pub struct BucketInvokeArg {
    pub bucket_ref: BucketRef,
    pub action: BucketAction,
    pub args: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Decode, Encode)]
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

#[derive(Clone, Copy, Debug, Decode, Encode)]
pub enum BucketAction {
    Create,
    GetResourceAddress,
    GetResourceType,
    GetAmount,
    Take,
    Burn,
}

#[derive(Clone, Debug, Decode, Encode)]
pub struct BucketBurnArg {
    pub bucket_id: BucketId,
}

// -------------------------------- Workspace -------------------------------- //
#[derive(Clone, Copy, Debug, Decode, Encode)]
pub enum WorkspaceAction {
    Put,
    PutLastInstructionOutput,
    Take,
    ListBuckets,
}

#[derive(Clone, Debug, Decode, Encode)]
pub struct WorkspaceInvokeArg {
    pub action: WorkspaceAction,
    pub args: Vec<Vec<u8>>,
}

// -------------------------------- NonFungible -------------------------------- //
#[derive(Clone, Debug, Decode, Encode)]
pub struct NonFungibleInvokeArg {
    pub address: NonFungibleAddress,
    pub action: NonFungibleAction,
    pub args: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Decode, Encode)]
pub enum NonFungibleAction {
    GetData,
    GetMutableData,
}

// -------------------------------- Consensus -------------------------------- //
#[derive(Clone, Debug, Decode, Encode)]
pub struct ConsensusInvokeArg {
    pub action: ConsensusAction,
}

#[derive(Clone, Debug, Decode, Encode)]
pub enum ConsensusAction {
    GetCurrentEpoch,
}
