//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::*;

#[template]
mod template {
    use super::*;

    pub struct Shenanigans {
        vault: Vault,
    }

    impl Shenanigans {
        pub fn new() -> Component<Shenanigans> {
            let resource = ResourceBuilder::non_fungible().mintable(rule!(allow_all)).build();

            Component::new(Self {
                vault: Vault::new_empty(resource),
            })
            .with_access_rules(AccessRules::allow_all())
            .create()
        }

        pub fn mint_different_resource_type(&mut self) -> Bucket {
            ResourceManager::get(self.vault.resource_address()).mint_fungible(Amount(1000))
        }
    }
}
