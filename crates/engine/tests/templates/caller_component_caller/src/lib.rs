//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::*;

/// Caller template: performs cross-component calls into a callee's methods.
#[template]
mod caller {
    use super::*;

    pub struct Caller;

    impl Caller {
        pub fn new() -> Component<Self> {
            let access_rules = ComponentAccessRules::new()
                .method("call_bar", rule!(allow_all))
                .method("call_ping", rule!(allow_all))
                .method("call_open_bar", rule!(allow_all))
                .method("withdraw_from", rule!(allow_all))
                .method("withdraw_via", rule!(allow_all))
                .default(rule!(deny_all));

            Component::new(Caller)
                .with_access_rules(access_rules)
                .with_owner_rule(OwnerRule::None)
                .create()
        }

        pub fn call_bar(&self, callee: ComponentAddress) {
            ComponentManager::get(callee).invoke("bar", args![]);
        }

        pub fn call_open_bar(&self, callee: ComponentAddress) {
            ComponentManager::get(callee).invoke("open_bar", args![]);
        }

        pub fn call_ping(&self, callee: ComponentAddress) -> u64 {
            ComponentManager::get(callee).call("ping", args![])
        }

        pub fn withdraw_from(&self, holder: ComponentAddress) {
            ComponentManager::get(holder).invoke("withdraw_once", args![]);
        }

        /// Forwards through `next`, so the holder's frame is entered by `next` rather than by this
        /// component.
        pub fn withdraw_via(&self, next: ComponentAddress, holder: ComponentAddress) {
            ComponentManager::get(next).invoke("withdraw_from", args![holder]);
        }

        // A static (template-function) caller has no component instance, only a template identity.
        pub fn withdraw_from_static(holder: ComponentAddress) {
            ComponentManager::get(holder).invoke("withdraw_once", args![]);
        }
    }
}
