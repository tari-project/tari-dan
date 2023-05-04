//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::{auth::AccessRules, Hash};

pub trait ComponentInterface {
    type Component: ComponentInstanceInterface;

    fn create(self) -> Self::Component
    where Self: Sized {
        // TODO: What should happen if you create a component without access rules?
        self.create_with_options(AccessRules::new(), None)
    }

    fn create_with_options(self, access_rules: AccessRules, component_id: Option<Hash>) -> Self::Component;
}

pub trait ComponentInstanceInterface {
    fn set_access_rules(self, rules: AccessRules) -> Self;
}
