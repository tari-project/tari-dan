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

use tari_template_abi::rust::collections::HashMap;
use tari_template_lib::prelude::*;

#[template]
mod account_template {
    use super::*;

    pub struct Account {
        // TODO: Lazy key value map/store
        vaults: HashMap<ResourceAddress, Vault>,
    }

    impl Account {
        pub fn create(owner_token: NonFungibleAddress) -> AccountComponent {
            let rules = AccessRules::new()
                .add_method_rule("balance", AccessRule::AllowAll)
                .add_method_rule("get_balances", AccessRule::AllowAll)
                .add_method_rule("deposit", AccessRule::AllowAll)
                .add_method_rule("deposit_all", AccessRule::AllowAll)
                .add_method_rule("get_non_fungible_ids", AccessRule::AllowAll)
                .default(AccessRule::Restricted(Require(owner_token)));

            Self::create_with_rules(rules)
        }

        pub fn create_with_rules(access_rules: AccessRules) -> AccountComponent {
            Self { vaults: HashMap::new() }.create_with_access_rules(access_rules)
        }

        // #[access_rule(allow_all)]
        pub fn balance(&self, resource: ResourceAddress) -> Amount {
            self.get_vault(resource)
                .map(|v| v.balance())
                .unwrap_or_else(Amount::zero)
        }

        // #[access_rule(requires(owner_badge))]
        pub fn withdraw(&mut self, resource: ResourceAddress, amount: Amount) -> Bucket {
            let v = self
                .get_vault_mut(resource)
                .expect("This account does not have any of that resource");

            v.withdraw(amount)
        }

        // #[access_rules(requires(owner_badge))]
        pub fn withdraw_non_fungible(&mut self, resource: ResourceAddress, nf_id: NonFungibleId) -> Bucket {
            let v = self
                .get_vault_mut(resource)
                .expect("This account does not have any of that resource");

            v.withdraw_non_fungibles(Some(nf_id))
        }

        // #[access_rules(allow_all)]
        pub fn deposit(&mut self, bucket: Bucket) {
            let resource_address = bucket.resource_address();
            let vault_mut = self
                .vaults
                .entry(resource_address)
                .or_insert_with(|| Vault::new_empty(resource_address));
            vault_mut.deposit(bucket);
        }

        pub fn deposit_all(&mut self, buckets: Vec<Bucket>) {
            for bucket in buckets {
                self.deposit(bucket);
            }
        }

        // #[access_rules(require(owner_badge))]
        pub fn get_non_fungible_ids(&self, resource: ResourceAddress) -> Vec<NonFungibleId> {
            let v = self
                .get_vault(resource)
                .unwrap_or_else(|| panic!("No vault for resource {}", resource));
            v.get_non_fungible_ids()
        }

        fn get_vault(&self, resource: ResourceAddress) -> Option<&Vault> {
            self.vaults.get(&resource)
        }

        fn get_vault_mut(&mut self, resource: ResourceAddress) -> Option<&mut Vault> {
            self.vaults.get_mut(&resource)
        }

        pub fn get_balances(&self) -> Vec<(ResourceAddress, Amount)> {
            self.vaults.iter().map(|(k, v)| (*k, v.balance())).collect()
        }
    }
}
