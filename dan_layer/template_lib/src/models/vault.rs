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

use serde::{Deserialize, Serialize};
use tari_bor::BorTag;
use tari_template_abi::{
    call_engine,
    rust::{
        collections::BTreeSet,
        fmt,
        fmt::{Display, Formatter},
        str::FromStr,
    },
    EngineOp,
};
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::{BinaryTag, EntityId, KeyParseError, NonFungible, ObjectKey, Proof, ProofAuth};
use crate::{
    args::{
        ConfidentialRevealArg,
        InvokeResult,
        PayFeeArg,
        VaultAction,
        VaultCreateProofByFungibleAmountArg,
        VaultCreateProofByNonFungiblesArg,
        VaultInvokeArg,
        VaultWithdrawArg,
    },
    models::{Amount, Bucket, ConfidentialWithdrawProof, NonFungibleId, ResourceAddress},
    newtype_struct_serde_impl,
    prelude::ResourceType,
    resource::ResourceManager,
};

const TAG: u64 = BinaryTag::VaultId as u64;

/// A vault's unique identification in the Tari network
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct VaultId(#[cfg_attr(feature = "ts", ts(type = "string"))] BorTag<ObjectKey, TAG>);

impl VaultId {
    pub const fn new(key: ObjectKey) -> Self {
        Self(BorTag::new(key))
    }

    pub fn from_hex(hex: &str) -> Result<Self, KeyParseError> {
        let key = ObjectKey::from_hex(hex)?;
        Ok(Self::new(key))
    }

    pub fn as_object_key(&self) -> &ObjectKey {
        self.0.inner()
    }

    pub fn entity_id(&self) -> EntityId {
        self.0.inner().as_entity_id()
    }
}

impl From<ObjectKey> for VaultId {
    fn from(key: ObjectKey) -> Self {
        Self::new(key)
    }
}

impl Display for VaultId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "vault_{}", *self.0)
    }
}

impl AsRef<[u8]> for VaultId {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl FromStr for VaultId {
    type Err = KeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.strip_prefix("vault_").unwrap_or(s);
        Self::from_hex(s)
    }
}

impl TryFrom<&[u8]> for VaultId {
    type Error = KeyParseError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let key = ObjectKey::try_from(value)?;
        Ok(Self::new(key))
    }
}

newtype_struct_serde_impl!(VaultId, BorTag<ObjectKey, TAG>);

/// Encapsulates all the ways that a vault can be referenced
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

impl Display for VaultRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            VaultRef::Vault { address, .. } => write!(f, "Vaults({})", address),
            VaultRef::Ref(id) => write!(f, "Ref({})", id),
        }
    }
}

/// A permanent container of resources. Vaults live after the end of a transaction.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Vault {
    vault_id: VaultId,
}

impl Vault {
    /// Returns a new vault with an empty balance
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

    /// Returns a new vault that will contain all the tokens from the provided bucket.
    /// The bucket will be empty after the call
    pub fn from_bucket(bucket: Bucket) -> Self {
        let resource_address = bucket.resource_address();
        let vault = Self::new_empty(resource_address);
        vault.deposit(bucket);
        vault
    }

    /// Deposit all the tokens from the provided bucket into the vault.
    /// The bucket will be empty after the call.
    /// It will panic if the tokens in the bucket are from a different resource than the ones in the vault
    pub fn deposit(&self, bucket: Bucket) {
        let result: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::Deposit,
            args: invoke_args![bucket.id()],
        });

        result.decode::<()>().expect("deposit failed");
    }

    /// Withdraw an `amount` of tokens from the vault into a new bucket.
    pub fn withdraw<T: Into<Amount>>(&self, amount: T) -> Bucket {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::Withdraw,
            args: invoke_args![VaultWithdrawArg::Fungible { amount: amount.into() }],
        });

        resp.decode().expect("failed to decode Bucket")
    }

    /// Withdraw a single non-fungible token from the vault into a new bucket.
    /// It will panic if the vault does not contain the specified non-fungible token
    pub fn withdraw_non_fungible(&self, id: NonFungibleId) -> Bucket {
        self.withdraw_non_fungibles(Some(id))
    }

    /// Withdraw multiple non-fungible tokens from the vault into a new bucket.
    /// It will panic if the vault does not contain the specified non-fungible tokens
    pub fn withdraw_non_fungibles<I: IntoIterator<Item = NonFungibleId>>(&self, ids: I) -> Bucket {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::Withdraw,
            args: invoke_args![VaultWithdrawArg::NonFungible {
                ids: ids.into_iter().collect()
            }],
        });

        resp.decode().expect("failed to decode Bucket")
    }

    /// Withdraws an amount (specified in the `proof`) of confidential tokens from the vault into a new bucket.
    /// It will panic if the proof is invalid or there are not enough tokens in the vault
    pub fn withdraw_confidential(&self, proof: ConfidentialWithdrawProof) -> Bucket {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::Withdraw,
            args: invoke_args![VaultWithdrawArg::Confidential { proof: Box::new(proof) }],
        });

        resp.decode().expect("failed to decode Bucket")
    }

    /// Withdraws all fungible, non-fungible and revealed confidential amounts from the vault into a new bucket.
    /// NOTE: blinded confidential amounts are not withdrawn as these require a `ConfidentialWithdrawProof`.
    pub fn withdraw_all(&mut self) -> Bucket {
        self.withdraw(self.balance())
    }

    /// Returns how many tokens this vault holds
    pub fn balance(&self) -> Amount {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::GetBalance,
            args: invoke_args![],
        });

        resp.decode().expect("failed to decode Amount")
    }

    /// Returns how many tokens this vault holds
    pub fn locked_balance(&self) -> Amount {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::GetLockedBalance,
            args: invoke_args![],
        });

        resp.decode().expect("failed to decode Amount")
    }

    /// Returns how many Pederson commitments (related to confidential balances) this vault holds
    pub fn commitment_count(&self) -> u32 {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::GetCommitmentCount,
            args: invoke_args![],
        });

        resp.decode().expect("failed to decode commitment count")
    }

    /// Returns the IDs of all the non-fungible this vault
    pub fn get_non_fungible_ids(&self) -> Vec<NonFungibleId> {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::GetNonFungibleIds,
            args: invoke_args![],
        });

        resp.decode()
            .expect("get_non_fungible_ids returned invalid non fungible ids")
    }

    /// Returns all the non-fungibles in this vault
    pub fn get_non_fungibles(&self) -> Vec<NonFungible> {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::GetNonFungibles,
            args: invoke_args![],
        });

        resp.decode().expect("get_non_fungibles returned invalid non fungibles")
    }

    /// Returns the resource address of the tokens that this vault holds
    pub fn resource_address(&self) -> ResourceAddress {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::GetResourceAddress,
            args: invoke_args![],
        });

        resp.decode()
            .expect("GetResourceAddress returned invalid resource address")
    }

    /// Returns the the type of resource that this vault holds
    pub fn resource_type(&self) -> ResourceType {
        ResourceManager::get(self.resource_address()).resource_type()
    }

    /// Returns a new bucket with revealed funds, specified by the `proof`.
    /// The amount of tokens will not change, only how many of those tokens will be known by everyone
    pub fn reveal_confidential(&self, proof: ConfidentialWithdrawProof) -> Bucket {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::ConfidentialReveal,
            args: invoke_args![ConfidentialRevealArg { proof }],
        });

        Bucket::from_id(resp.decode().expect("reveal_confidential returned invalid bucket"))
    }

    /// Pay a transaction fee with the funds present in the vault.
    /// Note that the vault must hold native Tari tokens to perform this operation
    pub fn pay_fee(&self, amount: Amount) {
        let _resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::PayFee,
            args: invoke_args![PayFeeArg { amount, proof: None }],
        });
    }

    /// Pay a transaction fee with the confidential funds present in the vault.
    /// Note that the vault must hold native Tari tokens to perform this operation
    pub fn pay_fee_confidential(&self, proof: ConfidentialWithdrawProof) {
        let _resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::PayFee,
            args: invoke_args![PayFeeArg {
                amount: Amount::zero(),
                proof: Some(proof)
            }],
        });
    }

    /// Deposit an amount (specified in the `proof`) of confidential tokens into the vault.
    /// It will panic if the proof is invalid or the resource of the proof is not the same as the one in the vault
    pub fn join_confidential(&self, proof: ConfidentialWithdrawProof) {
        let bucket = self.withdraw_confidential(proof);
        self.deposit(bucket);
    }

    /// Create a new proof that allows the holder to access the tokens in the vault with future instructions.
    /// The tokens will be locked during the lifespan of the transaction until the proof is destroyed.
    pub fn authorize(&self) -> ProofAuth {
        let proof = self.create_proof();
        ProofAuth { id: proof.id() }
    }

    /// Create a new proof that allows the holder to access the tokens in the vault with future instructions.
    /// It will execute the provided function after the proof is generated.
    /// The tokens will be locked during the lifespan of the transaction until the proof is destroyed
    pub fn authorize_with<F: FnOnce() -> R, R>(&self, f: F) -> R {
        let _auth = self.authorize();
        f()
    }

    /// Returns a new proof that demonstrates ownership of all the vault's tokens.
    /// The tokens will be locked during the lifespan of the transaction until the proof is destroyed.
    /// Used mostly for cross-component calls
    pub fn create_proof(&self) -> Proof {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::CreateProofByResource,
            args: invoke_args![],
        });

        resp.decode().expect("CreateProofOfResource failed")
    }

    /// Returns a new proof that demonstrates ownership of a specific amount of tokens.
    /// The tokens will be locked during the lifespan of the transaction until the proof is destroyed.
    /// Used mostly for cross-component calls
    pub fn create_proof_by_amount(&self, amount: Amount) -> Proof {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::CreateProofByFungibleAmount,
            args: invoke_args![VaultCreateProofByFungibleAmountArg { amount }],
        });

        resp.decode().expect("CreateProofByFungibleAmount failed")
    }

    /// Returns a new proof that demonstrates ownership of a specific set of non-fungibles.
    /// The tokens will be locked during the lifespan of the transaction until the proof is destroyed.
    /// Used mostly for cross-component calls
    pub fn create_proof_by_non_fungible_ids(&self, ids: BTreeSet<NonFungibleId>) -> Proof {
        let resp: InvokeResult = call_engine(EngineOp::VaultInvoke, &VaultInvokeArg {
            vault_ref: self.vault_ref(),
            action: VaultAction::CreateProofByNonFungibles,
            args: invoke_args![VaultCreateProofByNonFungiblesArg { ids }],
        });

        resp.decode().expect("CreateProofByNonFungibles failed")
    }

    pub fn vault_id(&self) -> VaultId {
        self.vault_id
    }

    fn vault_ref(&self) -> VaultRef {
        VaultRef::Ref(self.vault_id)
    }

    pub fn for_test(vault_id: VaultId) -> Self {
        Self { vault_id }
    }
}
