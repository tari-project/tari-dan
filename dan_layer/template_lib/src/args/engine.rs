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

use tari_template_abi::{
    decode,
    encode,
    rust::{fmt::Display, io},
    Decode,
    Encode,
};

use crate::{
    models::{Amount, BucketId, ComponentAddress, Metadata, ResourceAddress, VaultRef},
    resource::ResourceType,
};

#[derive(Debug, Clone, Encode, Decode)]
pub struct EmitLogArg {
    pub message: String,
    pub level: LogLevel,
}

#[derive(Debug, Clone, Copy, Encode, Decode)]
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

#[derive(Debug, Clone, Encode, Decode)]
pub struct ComponentInvokeArg {
    pub component_ref: ComponentRef,
    pub action: ComponentAction,
    pub args: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Decode, Encode)]
pub enum ComponentAction {
    Get,
    Create,
    SetState,
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

impl From<ResourceAddress> for ResourceRef {
    fn from(addr: ResourceAddress) -> Self {
        ResourceRef::Ref(addr)
    }
}

#[derive(Clone, Debug, Decode, Encode)]
pub enum ResourceAction {
    Mint,
    Burn,
    Deposit,
    Withdraw,
    Update,
}

#[derive(Clone, Debug, Decode, Encode)]
pub enum MintResourceArg {
    Fungible { amount: Amount, metadata: Metadata },
    NonFungible { token_ids: Vec<u64>, metadata: Metadata },
    // Confidential
}
impl MintResourceArg {
    pub fn to_resource_type(&self) -> ResourceType {
        match self {
            MintResourceArg::Fungible { .. } => ResourceType::Fungible,
            MintResourceArg::NonFungible { .. } => ResourceType::NonFungible,
        }
    }
}

#[derive(Clone, Debug, Decode, Encode)]
pub struct InvokeResult {
    output: Vec<u8>,
}

impl InvokeResult {
    pub fn encode<T: Encode>(output: &T) -> io::Result<Self> {
        let output = encode(output)?;
        Ok(Self { output })
    }

    pub fn decode<T: Decode>(&self) -> io::Result<T> {
        decode(&self.output)
    }

    pub fn unwrap_decoded<T: Decode>(&self) -> T {
        self.decode().unwrap()
    }

    pub fn unit() -> Self {
        Self { output: vec![] }
    }
}

#[derive(Clone, Debug, Decode, Encode)]
pub struct VaultInvokeArg {
    pub vault_ref: VaultRef,
    pub action: VaultAction,
    pub args: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Decode, Encode)]
pub enum VaultAction {
    Create,
    Deposit,
    WithdrawFungible,
    GetBalance,
    GetResourceAddress,
}

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
    Take,
    Drop,
}

pub enum WorkspaceAction {
    Put,
    PutLastInstructionOutput,
    Take,
}
