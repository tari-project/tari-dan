//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::*;

#[template]
mod dangling_template {
    use super::*;

    pub struct Reentrancy {
        vault: Option<Vault>,
        is_allowed: bool,
    }

    impl Reentrancy {
        pub fn with_bucket(bucket: Bucket) -> Component<Self> {
            Component::new(Self {
                vault: Some(Vault::from_bucket(bucket)),
                is_allowed: true,
            })
            .with_access_rules(AccessRules::allow_all())
            .create()
        }

        pub fn new() -> Component<Self> {
            Component::new(Self {
                vault: None,
                is_allowed: true,
            })
            .with_access_rules(AccessRules::allow_all())
            .create()
        }

        pub fn withdraw(&mut self, amount: Amount) -> Bucket {
            self.vault.as_mut().unwrap().withdraw(amount)
        }

        pub fn deposit(&mut self, bucket: Bucket) {
            self.vault.as_mut().unwrap().deposit(bucket)
        }

        pub fn get_balance(&self) -> Amount {
            self.vault.as_ref().unwrap().balance()
        }

        pub fn reentrant_withdraw(&mut self, amount: Amount) -> Bucket {
            let bucket1 = self.withdraw(amount);
            let bucket2 = ComponentManager::current().call("withdraw", args![amount]);
            self.deposit(bucket1);
            bucket2
        }

        pub fn reentrant_access(&mut self) {
            self.is_allowed = false;
            // The component state is not yet updated, so this would succeed if the engine allowed it
            ComponentManager::current().call("assert_is_allowed", args![])
        }

        pub fn reentrant_access_mut(&mut self) {
            self.is_allowed = false;
            // The component state is not yet updated, so this would succeed if the engine allowed it
            ComponentManager::current().call("reentrant_access_mut", args![])
        }

        pub fn reentrant_access_immutable(&self) {
            ComponentManager::current().call("assert_is_allowed", args![])
        }

        pub fn assert_is_allowed(&self) {
            assert!(self.is_allowed, "expected is_allowed == true")
        }
    }
}
