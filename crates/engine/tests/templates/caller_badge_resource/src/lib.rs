//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::*;

/// A resource whose withdraw rule is gated on the caller of whichever component holds it, held in a
/// vault by a component whose methods are open to everyone. Only the resource rule decides who may move
/// the tokens, so the gate applies to every holder rather than to one hand-written method gate.
#[template]
mod gated_resource {
    use super::*;

    pub struct GatedResource {
        vault: Vault,
    }

    impl GatedResource {
        pub fn new(gate: ComponentAddress) -> Component<Self> {
            let tokens = ResourceBuilder::public_fungible()
                .with_owner_rule(OwnerRule::None)
                .withdrawable(rule!(caller_component(gate)), LOCKED)
                .initial_supply(1000u32);

            Component::new(GatedResource {
                vault: Vault::from_bucket(tokens),
            })
            .with_owner_rule(OwnerRule::None)
            .with_access_rules(ComponentAccessRules::allow_all())
            .create()
        }

        /// Withdraws and immediately returns the tokens, so the call succeeds or fails purely on the
        /// resource's withdraw rule.
        pub fn withdraw_once(&mut self) {
            let bucket = self.vault.withdraw(Amount::new(1));
            self.vault.deposit(bucket);
        }

        pub fn balance(&self) -> Amount {
            self.vault.balance()
        }
    }
}
