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

use serde::{Deserialize, Serialize};
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
        ConfidentialWithdrawProof,
        Metadata,
        NonFungibleAddress,
        NonFungibleId,
        ResourceAddress,
        VaultRef,
    },
    prelude::{AccessRules, ConfidentialOutputProof},
    resource::ResourceType,
    Hash,
};

// -------------------------------- LOGS -------------------------------- //
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmitLogArg {
    pub message: String,
    pub level: LogLevel,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentInvokeArg {
    pub component_ref: ComponentRef,
    pub action: ComponentAction,
    pub args: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ComponentAction {
    Get,
    Create,
    SetState,
    SetAccessRules,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateComponentArg {
    pub module_name: String,
    pub encoded_state: Vec<u8>,
    pub access_rules: AccessRules,
    pub component_id: Option<Hash>,
}

// -------------------------------- Events -------------------------------- //

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmitEventArg {
    pub topic: String,
    pub payload: HashMap<String, String>,
}

// -------------------------------- Resource -------------------------------- //
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceInvokeArg {
    pub resource_ref: ResourceRef,
    pub action: ResourceAction,
    pub args: Vec<Vec<u8>>,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ResourceAction {
    GetTotalSupply,
    GetResourceType,
    GetNonFungible,
    Create,
    Mint,
    UpdateNonFungibleData,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MintArg {
    Fungible {
        amount: Amount,
    },
    NonFungible {
        tokens: HashMap<NonFungibleId, (Vec<u8>, Vec<u8>)>,
    },
    Confidential {
        proof: ConfidentialOutputProof,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateResourceArg {
    pub resource_type: ResourceType,
    pub token_symbol: String,
    pub metadata: Metadata,
    pub mint_arg: Option<MintArg>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MintResourceArg {
    pub mint_arg: MintArg,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceGetNonFungibleArg {
    pub id: NonFungibleId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceUpdateNonFungibleDataArg {
    pub id: NonFungibleId,
    pub data: Vec<u8>,
}

// -------------------------------- Vault -------------------------------- //
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultInvokeArg {
    pub vault_ref: VaultRef,
    pub action: VaultAction,
    pub args: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum VaultAction {
    Create,
    Deposit,
    Withdraw,
    WithdrawAll,
    GetBalance,
    GetResourceAddress,
    GetNonFungibleIds,
    GetCommitmentCount,
    ConfidentialReveal,
    PayFee,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum VaultWithdrawArg {
    Fungible { amount: Amount },
    NonFungible { ids: BTreeSet<NonFungibleId> },
    Confidential { proof: Box<ConfidentialWithdrawProof> },
}

// -------------------------------- Confidential -------------------------------- //
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfidentialRevealArg {
    pub proof: ConfidentialWithdrawProof,
}

// -------------------------------- Fees -------------------------------- //
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PayFeeArg {
    pub amount: Amount,
    pub proof: Option<ConfidentialWithdrawProof>,
}

// -------------------------------- Bucket -------------------------------- //
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BucketInvokeArg {
    pub bucket_ref: BucketRef,
    pub action: BucketAction,
    pub args: Vec<Vec<u8>>,
}

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

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum BucketAction {
    Create,
    GetResourceAddress,
    GetResourceType,
    GetAmount,
    Take,
    TakeConfidential,
    RevealConfidential,
    Burn,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BucketBurnArg {
    pub bucket_id: BucketId,
}

// -------------------------------- Workspace -------------------------------- //
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum WorkspaceAction {
    Put,
    PutLastInstructionOutput,
    Get,
    ListBuckets,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceInvokeArg {
    pub action: WorkspaceAction,
    pub args: Vec<Vec<u8>>,
}

// -------------------------------- NonFungible -------------------------------- //
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NonFungibleInvokeArg {
    pub address: NonFungibleAddress,
    pub action: NonFungibleAction,
    pub args: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NonFungibleAction {
    GetData,
    GetMutableData,
}

// -------------------------------- Consensus -------------------------------- //
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsensusInvokeArg {
    pub action: ConsensusAction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ConsensusAction {
    GetCurrentEpoch,
}

// -------------------------------- GenerateRandom -------------------------------- //
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenerateRandomInvokeArg {
    pub action: GenerateRandomAction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GenerateRandomAction {
    GetRandomBytes { len: u32 },
}

// -------------------------------- CallerContext -------------------------------- //
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallerContextInvokeArg {
    pub action: CallerContextAction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CallerContextAction {
    GetCallerPublicKey,
}
