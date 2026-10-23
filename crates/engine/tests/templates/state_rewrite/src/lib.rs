//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::*;

#[template]
mod template {
    use super::*;

    /// `a` and `b` follow `refs`, so the state's reference list holds them after every entry of `refs`.
    pub struct StateRewrite {
        refs: Vec<ResourceAddress>,
        a: ResourceAddress,
        b: ResourceAddress,
        vaults: Vec<Vault>,
    }

    impl StateRewrite {
        pub fn new(n: u32) -> Component<Self> {
            let a = ResourceBuilder::public_fungible().build();
            let b = ResourceBuilder::public_fungible().build();
            Component::new(Self {
                refs: vec![a; n as usize],
                a,
                b,
                vaults: Vec::new(),
            })
            .with_access_rules(AccessRules::allow_all())
            .create()
        }

        pub fn add_vaults(&mut self, count: u32) {
            for _ in 0..count {
                self.vaults.push(Vault::new_empty(self.a));
            }
        }

        /// Writes the state `times` times, reversing the order of `vaults` on each write. Every write
        /// holds the same vaults as the previous state, in the reverse order.
        pub fn reverse_vaults(&mut self, times: u32) {
            let manager = ComponentManager::get(CallerContext::current_component_address());
            for _ in 0..times {
                self.vaults.reverse();
                manager.set_state(&*self);
            }
        }

        /// Writes the state `times` times, alternating between `a` and `b`. With `replace_all`, each
        /// write fills `refs` with the address the previous `refs` did not hold; otherwise `refs`
        /// starts with both addresses on every write. Both modes write the same number of references
        /// and bytes.
        pub fn rewrite(&mut self, replace_all: bool, times: u32) {
            let manager = ComponentManager::get(CallerContext::current_component_address());
            let n = self.refs.len();
            for i in 0..times {
                let (head, tail) = if i % 2 == 0 { (self.b, self.a) } else { (self.a, self.b) };
                self.refs.clear();
                if replace_all {
                    self.refs.resize(n, head);
                } else {
                    self.refs.push(head);
                    self.refs.resize(n, tail);
                }
                manager.set_state(&*self);
            }
        }
    }
}
