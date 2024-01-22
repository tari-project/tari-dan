//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::marker::PhantomData;

use crate::{
    auth::{ComponentAccessRules, OwnerRule},
    engine,
    models::{AddressAllocation, ComponentAddress},
    Hash,
};

/// Utility for building components inside templates
pub struct ComponentBuilder<T> {
    component: T,
    owner_rule: OwnerRule,
    access_rules: ComponentAccessRules,
    component_id: Option<Hash>,
    address_allocation: Option<AddressAllocation<ComponentAddress>>,
}

impl<T: serde::Serialize> ComponentBuilder<T> {
    /// Returns a new component builder for the specified data
    fn new(component: T) -> Self {
        Self {
            component,
            owner_rule: OwnerRule::default(),
            access_rules: ComponentAccessRules::new(),
            component_id: None,
            address_allocation: None,
        }
    }

    /// Use an allocated address for the component.
    pub fn with_address_allocation(mut self, allocation: AddressAllocation<ComponentAddress>) -> Self {
        self.address_allocation = Some(allocation);
        self
    }

    /// Sets up who will be the owner of the component.
    /// Component owners are the only ones allowed to update the component's access rules after creation
    pub fn with_owner_rule(mut self, owner_rule: OwnerRule) -> Self {
        self.owner_rule = owner_rule;
        self
    }

    /// Sets up who can access each of the component's methods
    pub fn with_access_rules(mut self, access_rules: ComponentAccessRules) -> Self {
        self.access_rules = access_rules;
        self
    }

    /// Sets up the ID of the component, which must be unique for each component of the template
    pub fn with_component_id(mut self, component_id: Hash) -> Self {
        self.component_id = Some(component_id);
        self
    }

    /// Creates the new component and returns it
    pub fn create(self) -> Component<T> {
        let address = engine().create_component(
            self.component,
            self.owner_rule,
            self.access_rules,
            self.component_id,
            self.address_allocation,
        );
        Component::from_address(address)
    }
}

/// A newly created component, typically used as a return value from template constructor functions
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Component<T> {
    address: ComponentAddress,
    #[serde(skip)]
    _component: PhantomData<T>,
}

impl<T: serde::Serialize> Component<T> {
    /// Returns a new component builder for the specified data
    #[allow(clippy::new_ret_no_self)]
    pub fn new(component: T) -> ComponentBuilder<T> {
        ComponentBuilder::new(component)
    }

    /// Creates the new component and returns it
    pub fn create(component: T) -> Component<T> {
        Self::new(component).create()
    }

    /// Creates a new component with the specified address
    fn from_address(address: ComponentAddress) -> Self {
        Self {
            address,
            _component: PhantomData,
        }
    }

    /// Returns the address of the component
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
