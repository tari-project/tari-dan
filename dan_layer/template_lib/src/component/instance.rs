//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::marker::PhantomData;

use crate::{
    auth::{ComponentAccessRules, OwnerRule},
    engine,
    models::ComponentAddress,
    Hash,
};

pub struct ComponentBuilder<T> {
    component: T,
    owner_rule: OwnerRule,
    access_rules: ComponentAccessRules,
    component_id: Option<Hash>,
}

impl<T: serde::Serialize> ComponentBuilder<T> {
    fn new(component: T) -> Self {
        Self {
            component,
            owner_rule: OwnerRule::default(),
            access_rules: ComponentAccessRules::new(),
            component_id: None,
        }
    }

    pub fn with_owner_rule(mut self, owner_rule: OwnerRule) -> Self {
        self.owner_rule = owner_rule;
        self
    }

    pub fn with_access_rules(mut self, access_rules: ComponentAccessRules) -> Self {
        self.access_rules = access_rules;
        self
    }

    pub fn with_component_id(mut self, component_id: Hash) -> Self {
        self.component_id = Some(component_id);
        self
    }

    pub fn create(self) -> Component<T> {
        let address = engine().create_component(self.component, self.owner_rule, self.access_rules, self.component_id);
        Component::from_address(address)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Component<T> {
    address: ComponentAddress,
    #[serde(skip)]
    _component: PhantomData<T>,
}

impl<T: serde::Serialize> Component<T> {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(component: T) -> ComponentBuilder<T> {
        ComponentBuilder::new(component)
    }

    pub fn create(component: T) -> Component<T> {
        Self::new(component).create()
    }

    fn from_address(address: ComponentAddress) -> Self {
        Self {
            address,
            _component: PhantomData,
        }
    }

    pub fn address(&self) -> &ComponentAddress {
        &self.address
    }
}

#[cfg(test)]
mod tests {
    use tari_bor::{decode, encode};

    use super::*;

    #[test]
    fn it_serializes_as_a_component_address() {
        decode::<ComponentAddress>(
            &encode(&Component::<u32>::from_address(ComponentAddress::new(Hash::default()))).unwrap(),
        )
        .unwrap();
    }
}
