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

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use tari_bor::BorTag;
use tari_template_abi::{
    call_engine,
    rust::{
        fmt,
        fmt::{Display, Formatter},
    },
    EngineOp,
};

use super::BinaryTag;
use crate::{
    args::{ConfidentialRevealArg, InvokeResult, PayFeeArg, VaultAction, VaultInvokeArg, VaultWithdrawArg},
    hash::HashParseError,
    models::{Amount, Bucket, ConfidentialWithdrawProof, NonFungibleId, ResourceAddress},
    newtype_struct_serde_impl,
    Hash,
};

const TAG: u64 = BinaryTag::VaultId as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VaultId(BorTag<Hash, TAG>);

impl VaultId {
    pub const fn new(address: Hash) -> Self {
        Self(BorTag::new(address))
    }

    pub fn hash(&self) -> &Hash {
        &self.0
    }

    pub fn from_hex(hex: &str) -> Result<Self, HashParseError> {
        let hash = Hash::from_hex(hex)?;
        Ok(Self::new(hash))
    }
}

impl From<Hash> for VaultId {
    fn from(address: Hash) -> Self {
        Self::new(address)
    }
}

impl Display for VaultId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "vault_{}", *self.0)
    }
}

impl AsRef<[u8]> for VaultId {
    fn as_ref(&self) -> &[u8] {
        self.hash()
    }
}

impl FromStr for VaultId {
    type Err = HashParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.strip_prefix("vault_").ok_or(HashParseError)?;
        let hash = Hash::from_hex(s)?;
        Ok(Self::new(hash))
    }
}

impl TryFrom<Vec<u8>> for VaultId {
    type Error = HashParseError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let hash = Hash::try_from(value)?;
        Ok(Self::new(hash))
    }
}

newtype_struct_serde_impl!(VaultId, BorTag<Hash, TAG>);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum VaultRef {
    Vault { address: ResourceAddress },
    Ref(VaultId),
}

impl VaultRef {
    pub fn resource_address(&self) -> Option<&ResourceAddress> {
        match self {
            VaultRef::Vault { address, .. } => Some(address),
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Vault {
    vault_id: VaultId,
}

impl Vault {
    pub fn new_empty(resource_address: ResourceAddress) -> Self {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: VaultRef::Vault {
                address: resource_address,
            },
            action: VaultAction::Create,
            args: args![],
        });

        Self {
            vault_id: resp.decode().unwrap(),
        }
    }

    pub fn from_bucket(bucket: Bucket) -> Self {
        let resource_address = bucket.resource_address();
        let mut vault = Self::new_empty(resource_address);
        vault.deposit(bucket);
        vault
    }

    pub fn deposit(&mut self, bucket: Bucket) {
        let result: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::Deposit,
            args: invoke_args![bucket.id()],
        });

        result.decode::<()>().expect("deposit failed");
    }

    pub fn withdraw(&mut self, amount: Amount) -> Bucket {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::Withdraw,
            args: invoke_args![VaultWithdrawArg::Fungible { amount }],
        });

        resp.decode().expect("failed to decode Bucket")
    }

    pub fn withdraw_non_fungibles<I: IntoIterator<Item = NonFungibleId>>(&mut self, ids: I) -> Bucket {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::Withdraw,
            args: invoke_args![VaultWithdrawArg::NonFungible {
                ids: ids.into_iter().collect()
            }],
        });

        resp.decode().expect("failed to decode Bucket")
    }

    pub fn withdraw_confidential(&mut self, proof: ConfidentialWithdrawProof) -> Bucket {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::Withdraw,
            args: invoke_args![VaultWithdrawArg::Confidential { proof: Box::new(proof) }],
        });

        resp.decode().expect("failed to decode Bucket")
    }

    pub fn withdraw_all(&mut self) -> Bucket {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::WithdrawAll,
            args: invoke_args![],
        });

        resp.decode().expect("failed to decode Bucket")
    }

    pub fn balance(&self) -> Amount {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::GetBalance,
            args: invoke_args![],
        });

        resp.decode().expect("failed to decode Amount")
    }

    pub fn commitment_count(&self) -> u32 {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::GetCommitmentCount,
            args: invoke_args![],
        });

        resp.decode().expect("failed to decode commitment count")
    }

    pub fn get_non_fungible_ids(&self) -> Vec<NonFungibleId> {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::GetNonFungibleIds,
            args: invoke_args![],
        });

        resp.decode()
            .expect("get_non_fungible_ids returned invalid non fungible ids")
    }

    pub fn resource_address(&self) -> ResourceAddress {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::GetResourceAddress,
            args: invoke_args![],
        });

        resp.decode()
            .expect("GetResourceAddress returned invalid resource address")
    }

    pub fn reveal_confidential(&mut self, proof: ConfidentialWithdrawProof) -> Bucket {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::ConfidentialReveal,
            args: invoke_args![ConfidentialRevealArg { proof }],
        });

        Bucket::from_id(resp.decode().expect("reveal_confidential returned invalid bucket"))
    }

    pub fn pay_fee(&mut self, amount: Amount) {
        let _resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::PayFee,
            args: invoke_args![PayFeeArg { amount, proof: None }],
        });
    }

    pub fn pay_fee_confidential(&mut self, proof: ConfidentialWithdrawProof) {
        let _resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::PayFee,
            args: invoke_args![PayFeeArg {
                amount: Amount::zero(),
                proof: Some(proof)
            }],
        });
    }

    pub fn join_confidential(&mut self, proof: ConfidentialWithdrawProof) {
        let bucket = self.withdraw_confidential(proof);
        self.deposit(bucket);
    }

    pub fn vault_id(&self) -> VaultId {
        self.vault_id
    }

    pub fn vault_ref(&self) -> VaultRef {
        VaultRef::Ref(self.vault_id)
    }

    pub fn for_test(vault_id: VaultId) -> Self {
        Self { vault_id }
    }
}
