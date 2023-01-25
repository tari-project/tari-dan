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

// The Github CI tries to fmt and check inside the templates folder
#![allow(clippy::all)]

use tari_template_lib::prelude::*;

#[template]
mod account_template {
    use std::collections::BTreeSet;

    use super::*;

    pub struct Account {
        // owner_address: RistrettoPublicKey,
        // TODO: Lazy key value map/store
        vaults: Vec<(ResourceAddress, Vault)>,
    }

    impl Account {
        pub fn new() -> Self {
            Self { vaults: Vec::new() }
        }

        fn get_vault(&self, resource: ResourceAddress) -> Option<&Vault> {
            self.vaults
                .iter()
                .find(|(addr, _)| *addr == resource)
                .map(|(_, vault)| vault)
        }

        fn get_vault_mut(&mut self, resource: ResourceAddress) -> Option<&mut Vault> {
            self.vaults
                .iter_mut()
                .find(|(addr, _)| *addr == resource)
                .map(|(_, vault)| vault)
        }

        pub fn balance(&self, resource: ResourceAddress) -> Amount {
            let v = self
                .get_vault(resource)
                .ok_or_else(|| format!("No vault for resource {}", resource))
                .unwrap();
            v.balance()
        }

        // #[access_rules(requires(owner_badge))]
        pub fn withdraw(&mut self, resource: ResourceAddress, amount: Amount) -> Bucket {
            let v = self
                .get_vault_mut(resource)
                .expect("This account does not have any of that resource");

            v.withdraw(amount)
        }

        // #[access_rules(allow_all)]
        pub fn deposit(&mut self, bucket: Bucket) {
            let resource_address = bucket.resource_address();
            if let Some(v) = self.get_vault_mut(resource_address) {
                v.deposit(bucket);
            } else {
                let mut new_vault = Vault::new_empty(resource_address);
                new_vault.deposit(bucket);
                self.vaults.push((resource_address, new_vault));
            }
        }

        // #[access_rules(require(owner_badge))]
        pub fn get_non_fungible_ids(&self, resource: ResourceAddress) -> BTreeSet<NonFungibleId> {
            let v = self
                .get_vault(resource)
                .ok_or_else(|| format!("No vault for resource {}", resource))
                .unwrap();
            v.get_non_fungible_ids()
        }

        // pub fn deposit_all_from_workspace(&mut self) {
        //     for bucket in WorkspaceManager::take_all_buckets() {
        //         debug(format!("bucket: {}", bucket));
        //         self.deposit(bucket);
        //     }
        // }
    }
}
