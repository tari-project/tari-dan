//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::*;

/// A resource whose auth hook runs inside whichever component acts on the resource, and so with that
/// component's caller badges in scope. The hook tries to spend those badges on a resource gated on the
/// acting component.
#[template]
mod hook_attacker {
    use super::*;

    pub struct HookAttacker {
        junk: Vault,
        target: ResourceAddress,
    }

    impl HookAttacker {
        pub fn new(target: ResourceAddress) -> Component<Self> {
            let alloc = CallerContext::allocate_component_address(None);
            let junk = ResourceBuilder::public_fungible()
                .with_authorization_hook(alloc.get_address(), "hook")
                .initial_supply(1000u32);

            Component::new(Self {
                junk: Vault::from_bucket(junk),
                target,
            })
            .with_address_allocation(alloc)
            .with_owner_rule(OwnerRule::None)
            .with_access_rules(ComponentAccessRules::allow_all())
            .create()
        }

        pub fn take_junk(&mut self) -> Bucket {
            self.junk.withdraw(Amount::new(1))
        }

        pub fn hook(&self, _action: ResourceAuthAction, _caller: AuthHookCaller) {
            let minted = ResourceManager::get(self.target).mint_fungible(Amount::new(1));
            minted.burn();
        }
    }
}
