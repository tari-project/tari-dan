//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::{auth::AccessRules, component::OwnedValue};

pub trait ComponentInterface {
    type Component: ComponentInstanceInterface;

    fn create(self) -> Self::Component
    where Self: Sized {
        // TODO: What should happen if you create a component without access rules?
        self.create_with_access_rules(AccessRules::new())
    }

    fn create_with_access_rules(self, access_rules: AccessRules) -> Self::Component;

    fn get_owned_values(&self) -> Vec<OwnedValue>;
}

pub trait ComponentInstanceInterface {
    fn set_access_rules(self, rules: AccessRules) -> Self;
}
