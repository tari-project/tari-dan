//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::marker::PhantomData;

use crate::{
    auth::{ComponentAccessRules, OwnerRule},
    caller_context::CallerContext,
    crypto::RistrettoPublicKeyBytes,
    engine,
    models::{AddressAllocation, ComponentAddress},
};

/// Utility for building components inside templates
pub struct ComponentBuilder<T> {
    component: T,
    owner_rule: OwnerRule,
    access_rules: ComponentAccessRules,
    public_key_address: Option<RistrettoPublicKeyBytes>,
    address_allocation: Option<AddressAllocation<ComponentAddress>>,
}

impl<T> ComponentBuilder<T> {
    /// Returns a new component builder for the specified data
    fn new(component: T) -> Self {
        Self {
            component,
            owner_rule: OwnerRule::default(),
            access_rules: ComponentAccessRules::new(),
            public_key_address: None,
            address_allocation: None,
        }
    }

    /// Use an allocated address for the component.
    pub fn with_address_allocation(mut self, allocation: AddressAllocation<ComponentAddress>) -> Self {
        self.address_allocation = Some(allocation);
        self
    }

    pub fn with_public_key_address(mut self, public_key: RistrettoPublicKeyBytes) -> Self {
        self.public_key_address = Some(public_key);
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
}

impl<T: serde::Serialize> ComponentBuilder<T> {
    /// Creates the new component and returns it
    pub fn create(self) -> Component<T> {
        if self.public_key_address.is_some() && self.address_allocation.is_some() {
            panic!("Cannot specify both a public key address and an address allocation");
        }

        let address_allocation = self
            .public_key_address
            // Allocate public key address is necessary
            .map(|pk| CallerContext::allocate_component_address(Some(pk)))
            .or(self.address_allocation);

        let address = engine().create_component(self.component, self.owner_rule, self.access_rules, address_allocation);
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
    use crate::models::ObjectKey;

    #[test]
    fn it_serializes_as_a_component_address() {
        decode::<ComponentAddress>(
            &encode(&Component::<u32>::from_address(ComponentAddress::new(
                ObjectKey::default(),
            )))
            .unwrap(),
        )
        .unwrap();
    }
}
