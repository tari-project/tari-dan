//  Copyright 2022. The Tari Project
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

use std::marker::PhantomData;

use tari_template_abi::{call_engine, Decode, Encode, EngineOp};

use crate::{
    args::{InvokeResult, VaultAction, VaultInvokeArg},
    models::{Amount, Bucket, ResourceAddress},
    resource::{ResourceDefinition, ResourceType},
    Hash,
};

pub type VaultId = (Hash, u32);

#[derive(Clone, Debug, Decode, Encode)]
pub enum VaultRef {
    Vault {
        address: ResourceAddress,
        resource_type: ResourceType,
    },
    Ref(VaultId),
}

impl VaultRef {
    pub fn resource_address(&self) -> Option<&ResourceAddress> {
        match self {
            VaultRef::Vault { address, .. } => Some(address),
            VaultRef::Ref(_) => None,
        }
    }

    pub fn resource_type(&self) -> Option<ResourceType> {
        match self {
            VaultRef::Vault { resource_type, .. } => Some(*resource_type),
            VaultRef::Ref(_) => None,
        }
    }

    pub fn vault_id(&self) -> Option<VaultId> {
        match self {
            VaultRef::Vault { .. } => None,
            VaultRef::Ref(id) => Some(*id),
        }
    }
}

#[derive(Clone, Debug, Decode, Encode)]
pub struct Vault<T> {
    vault_id: VaultId,
    _t: PhantomData<T>,
}

impl<T: ResourceDefinition> Vault<T> {
    pub fn new_empty(resource_address: ResourceAddress) -> Self {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: VaultRef::Vault {
                address: resource_address,
                resource_type: T::resource_type(),
            },
            action: VaultAction::Create,
            args: args![],
        })
        .expect("OP_CREATE_VAULT returned null");

        Self {
            vault_id: resp.decode().unwrap(),
            _t: PhantomData,
        }
    }

    pub fn from_bucket(bucket: Bucket<T>) -> Self {
        let mut vault = Self::new_empty(bucket.resource_address());
        vault.deposit(bucket);
        vault
    }

    pub fn deposit(&mut self, bucket: Bucket<T>) {
        call_engine::<_, ()>(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: VaultRef::Ref(self.vault_id()),
            action: VaultAction::Deposit,
            args: invoke_args![bucket.id()],
        })
        .expect("VaultInvoke returned null");
    }

    pub fn withdraw<A: Into<Amount>>(&mut self, amount: A) -> Bucket<T> {
        let amount = amount.into();
        assert!(
            amount.is_positive() && !amount.is_zero(),
            "Amount must be non-zero and positive"
        );
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: VaultRef::Ref(self.vault_id()),
            action: VaultAction::WithdrawFungible,
            args: invoke_args![amount],
        })
        .expect("VaultInvoke returned null");

        resp.decode().expect("failed to decode Bucket")
    }

    pub fn balance(&self) -> Amount {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: VaultRef::Ref(self.vault_id()),
            action: VaultAction::GetBalance,
            args: args![],
        })
        .expect("VaultInvoke returned null");

        resp.decode().expect("failed to decode Amount")
    }

    pub fn resource_address(&self) -> ResourceAddress {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: VaultRef::Ref(self.vault_id()),
            action: VaultAction::GetResourceAddress,
            args: invoke_args![],
        })
        .expect("GetResourceAddress returned null");

        resp.decode()
            .expect("GetResourceAddress returned invalid resource address")
    }

    pub fn vault_id(&self) -> VaultId {
        self.vault_id
    }
}
