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
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_template_lib::{
    args::VaultFreezeFlags,
    types::{
        Amount,
        NonFungibleId,
        ResourceAddress,
        ResourceType,
        VaultId,
        confidential::ConfidentialWithdrawProof,
        crypto::PedersenCommitmentBytes,
    },
};

use crate::{
    bucket::Bucket,
    proof::{ContainerRef, LockedResource, Proof},
    resource_container::{ConfidentialOutputEffects, ResourceContainer, ResourceError},
};

#[derive(
    Debug, Clone, minicbor::Encode, minicbor::Decode, minicbor::CborLen, Serialize, Deserialize, borsh::BorshSerialize,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Vault {
    #[n(0)]
    resource_container: ResourceContainer,
    #[n(1)]
    #[cbor(default)]
    #[serde(default)]
    freeze_flags: VaultFreezeFlags,
}

impl Vault {
    pub fn new(resource: ResourceContainer) -> Self {
        Self {
            resource_container: resource,
            freeze_flags: VaultFreezeFlags::empty(),
        }
    }

    pub fn deposit(&mut self, bucket: Bucket) -> Result<(), ResourceError> {
        self.resource_container.deposit(bucket.into_resource_container())?;
        Ok(())
    }

    pub fn withdraw<T: Into<Amount>>(&mut self, amount: T) -> Result<ResourceContainer, ResourceError> {
        self.resource_container.withdraw(amount.into())
    }

    pub fn withdraw_non_fungibles(
        &mut self,
        ids: &BTreeSet<NonFungibleId>,
    ) -> Result<ResourceContainer, ResourceError> {
        self.resource_container.withdraw_by_ids(ids)
    }

    pub fn withdraw_confidential(
        &mut self,
        proof: ConfidentialWithdrawProof,
        view_key: Option<&RistrettoPublicKey>,
    ) -> Result<(ResourceContainer, ConfidentialOutputEffects), ResourceError> {
        self.resource_container.withdraw_confidential(proof, view_key)
    }

    pub fn recall_all(&mut self) -> Result<ResourceContainer, ResourceError> {
        self.resource_container.withdraw_all()
    }

    pub fn recall_confidential<T: Into<Amount>>(
        &mut self,
        commitments: &BTreeSet<PedersenCommitmentBytes>,
        revealed_amount: T,
    ) -> Result<ResourceContainer, ResourceError> {
        self.resource_container
            .recall_confidential_commitments(commitments, revealed_amount.into())
    }

    pub fn set_freeze(&mut self, flags: VaultFreezeFlags) {
        self.freeze_flags = flags;
    }

    pub fn unfreeze(&mut self) {
        self.set_freeze(VaultFreezeFlags::empty());
    }

    pub fn freeze_flags(&self) -> VaultFreezeFlags {
        self.freeze_flags
    }

    pub fn balance(&self) -> Amount {
        self.resource_container.unlocked_amount()
    }

    pub fn locked_balance(&self) -> Amount {
        self.resource_container.locked_amount()
    }

    /// Whether any funds in this vault are locked, e.g. by a proof. See
    /// [`ResourceContainer::has_locked_funds`] for why the balance alone cannot answer this.
    pub fn has_locked_funds(&self) -> bool {
        self.resource_container.has_locked_funds()
    }

    pub fn get_commitment_count(&self) -> u64 {
        self.resource_container.get_commitment_count()
    }

    pub fn get_confidential_commitments(&self) -> Option<&BTreeSet<PedersenCommitmentBytes>> {
        self.resource_container.get_confidential_commitments()
    }

    pub fn resource_address(&self) -> &ResourceAddress {
        self.resource_container.resource_address()
    }

    pub fn resource_type(&self) -> ResourceType {
        self.resource_container.resource_type()
    }

    pub fn get_non_fungible_ids(&self) -> &BTreeSet<NonFungibleId> {
        self.resource_container.non_fungible_token_ids()
    }

    pub fn resource_container_mut(&mut self) -> &mut ResourceContainer {
        &mut self.resource_container
    }

    pub fn lock_all(&mut self, vault_id: VaultId) -> Result<LockedResource, ResourceError> {
        let locked_resource = self.resource_container.lock_all()?;
        Ok(LockedResource::new(ContainerRef::Vault(vault_id), locked_resource))
    }

    pub fn lock_by_non_fungible_ids(
        &mut self,
        vault_id: VaultId,
        ids: BTreeSet<NonFungibleId>,
    ) -> Result<LockedResource, ResourceError> {
        let locked_resource = self.resource_container.lock_by_non_fungible_ids(ids)?;
        Ok(LockedResource::new(ContainerRef::Vault(vault_id), locked_resource))
    }

    pub fn lock_by_amount<T: Into<Amount>>(
        &mut self,
        vault_id: VaultId,
        amount: T,
    ) -> Result<LockedResource, ResourceError> {
        let locked_resource = self.resource_container.lock_by_amount(amount.into())?;
        Ok(LockedResource::new(ContainerRef::Vault(vault_id), locked_resource))
    }

    pub fn unlock(&mut self, proof: Proof) -> Result<(), ResourceError> {
        self.resource_container.unlock(proof.into_resource_container())
    }
}
