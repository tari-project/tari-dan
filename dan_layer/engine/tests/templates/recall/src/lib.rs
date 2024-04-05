//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use tari_template_lib::prelude::*;

#[template]
mod template {
    use std::collections::BTreeSet;

    use super::*;

    pub struct Recall {
        fungible: Vault,
        non_fungible: Vault,
        confidential: Vault,
    }

    impl Recall {
        pub fn new(
            confidential_supply: ConfidentialOutputStatement,
        ) -> (Component<Self>, ResourceAddress, ResourceAddress, ResourceAddress) {
            let fungible = ResourceBuilder::fungible()
                .initial_supply(Amount(1_000_000))
                .recallable(AccessRule::AllowAll)
                .build_bucket();
            let fungible_resource = fungible.resource_address();

            let non_fungible = ResourceBuilder::non_fungible()
                .mint_many_with(1..=10, |n| (NonFungibleId::from_u32(n), (&(), &())))
                .recallable(AccessRule::AllowAll)
                .build_bucket();
            let non_fungible_resource = non_fungible.resource_address();

            let confidential = ResourceBuilder::confidential()
                .initial_supply(confidential_supply)
                .recallable(AccessRule::AllowAll)
                .build_bucket();
            let confidential_resource = confidential.resource_address();

            let component = Component::new(Self {
                fungible: Vault::from_bucket(fungible),
                non_fungible: Vault::from_bucket(non_fungible),
                confidential: Vault::from_bucket(confidential),
            })
            .with_access_rules(AccessRules::allow_all())
            .create();

            (
                component,
                fungible_resource,
                non_fungible_resource,
                confidential_resource,
            )
        }

        pub fn withdraw_some(&mut self, confidential: ConfidentialWithdrawProof) -> (Bucket, Bucket, Bucket) {
            let fungible = self.fungible.withdraw(Amount(10));
            let non_fungible = self
                .non_fungible
                .withdraw_non_fungibles([NonFungibleId::from_u32(1), NonFungibleId::from_u32(2)]);
            let confidential = self.confidential.withdraw_confidential(confidential);
            (fungible, non_fungible, confidential)
        }

        pub fn recall_fungible_all(&mut self, vault_id: VaultId) {
            let bucket = ResourceManager::get(self.fungible.resource_address()).recall_fungible_all(vault_id);
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
            }
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
            commitments: BTreeSet<PedersonCommitmentBytes>,
            revealed_amount: Amount,
        ) {
            let bucket = ResourceManager::get(self.confidential.resource_address()).recall_confidential(
                vault_id,
                commitments,
                revealed_amount,
            );
            self.confidential.deposit(bucket);
        }

        pub fn get_balances(&self) -> (Amount, Amount, Amount) {
            (
                self.fungible.balance(),
                self.non_fungible.balance(),
                self.confidential.balance(),
            )
        }
    }
}
