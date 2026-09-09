//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use tari_template_abi::rust::collections::BTreeMap;
use tari_template_lib::prelude::*;

#[template]
mod template {
    use std::collections::BTreeSet;

    use super::*;

    pub struct Recall {
        fungible: Vault,
        non_fungible: Vault,
        confidential: Vault,
        stealth: Vault,
    }

    impl Recall {
        pub fn new(
            confidential_supply: ConfidentialOutputStatement,
            confidential_value_proofs: BTreeMap<PedersenCommitmentBytes, crypto::CommitmentValueProof>,
        ) -> (
            Component<Self>,
            ResourceAddress,
            ResourceAddress,
            ResourceAddress,
            ResourceAddress,
        ) {
            let fungible = ResourceBuilder::public_fungible()
                .recallable(rule!(allow_all), OWNER)
                .initial_supply(1_000_000u32);

            let fungible_resource = fungible.resource_address();

            let non_fungible = ResourceBuilder::non_fungible()
                .recallable(rule!(allow_all), OWNER)
                .initial_supply((1..=10).map(NonFungibleId::from_u32));
            let non_fungible_resource = non_fungible.resource_address();

            let confidential = ResourceBuilder::confidential()
                .recallable(rule!(allow_all), OWNER)
                .initial_supply_with_value_proofs(confidential_supply, confidential_value_proofs);
            let confidential_resource = confidential.resource_address();

            let stealth = ResourceBuilder::stealth()
                .recallable(rule!(allow_all), OWNER)
                .initial_supply(1_000_000u32);
            let stealth_resource = stealth.resource_address();

            let component = Component::new(Self {
                fungible: Vault::from_bucket(fungible),
                non_fungible: Vault::from_bucket(non_fungible),
                confidential: Vault::from_bucket(confidential),
                stealth: Vault::from_bucket(stealth),
            })
            .with_access_rules(AccessRules::allow_all())
            .create();

            (
                component,
                fungible_resource,
                non_fungible_resource,
                confidential_resource,
                stealth_resource,
            )
        }

        pub fn withdraw_some(&mut self, confidential: ConfidentialWithdrawProof) -> (Bucket, Bucket, Bucket, Bucket) {
            let fungible = self.fungible.withdraw(10u32);
            let non_fungible = self
                .non_fungible
                .withdraw_non_fungibles([NonFungibleId::from_u32(1), NonFungibleId::from_u32(2)]);
            let confidential = self.confidential.withdraw_confidential(confidential);
            let stealth = self.stealth.withdraw(10u32);
            (fungible, non_fungible, confidential, stealth)
        }

        pub fn recall_all(&mut self, vault_id: VaultId) {
            let bucket = ResourceManager::get(self.fungible.resource_address()).recall_all(vault_id);
            match bucket.resource_type() {
                ResourceType::Fungible => {
                    self.fungible.deposit(bucket);
                },
                ResourceType::NonFungible => {
                    self.non_fungible.deposit(bucket);
                },
                ResourceType::Confidential => {
                    self.confidential.deposit(bucket);
                },
                ResourceType::Stealth => {
                    self.stealth.deposit(bucket);
                },
            }
        }

        /// Recalls under this component's fungible resource rule and hands the bucket back to the caller, who
        /// decides where it goes.
        pub fn recall_fungible_to_bucket(&mut self, vault_id: VaultId, amount: Amount) -> Bucket {
            ResourceManager::get(self.fungible.resource_address()).recall_fungible_amount(vault_id, amount)
        }

        pub fn recall_fungible(&mut self, vault_id: VaultId, amount: Amount) {
            // NOTE: this call will only succeed if the resource is contained in the vault
            let bucket =
                ResourceManager::get(self.fungible.resource_address()).recall_fungible_amount(vault_id, amount);
            self.fungible.deposit(bucket);
        }

        pub fn recall_non_fungibles(&mut self, vault_id: VaultId, ids: BTreeSet<NonFungibleId>) {
            let bucket = ResourceManager::get(self.non_fungible.resource_address()).recall_non_fungibles(vault_id, ids);
            self.non_fungible.deposit(bucket);
        }

        pub fn recall_confidential(
            &mut self,
            vault_id: VaultId,
            commitments: BTreeSet<PedersenCommitmentBytes>,
            revealed_amount: Amount,
        ) {
            let bucket = ResourceManager::get(self.confidential.resource_address()).recall_confidential(
                vault_id,
                commitments,
                revealed_amount,
            );
            self.confidential.deposit(bucket);
        }

        pub fn recall_stealth(&mut self, vault_id: VaultId, revealed_amount: Amount) {
            let bucket =
                ResourceManager::get(self.stealth.resource_address()).recall_fungible_amount(vault_id, revealed_amount);
            self.stealth.deposit(bucket);
        }
    }
}
