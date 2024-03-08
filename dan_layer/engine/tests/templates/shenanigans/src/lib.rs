//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::*;

#[template]
mod template {
    use super::*;

    #[derive(Default)]
    pub struct Shenanigans {
        resource_address: Option<ResourceAddress>,
        component_address: Option<ComponentAddress>,
        vault: Option<Vault>,
        vault_copy: Option<Vault>,
        vault_ref: Option<tari_template_lib::models::VaultId>,
    }

    impl Shenanigans {
        pub fn dangling_vault() -> Self {
            let _vault = Vault::new_empty(CONFIDENTIAL_TARI_RESOURCE_ADDRESS);
            Self::default()
        }

        pub fn return_vault() -> Vault {
            Vault::new_empty(CONFIDENTIAL_TARI_RESOURCE_ADDRESS)
        }

        pub fn new() -> Self {
            Self::default()
        }

        pub fn with_vault() -> Self {
            let vault = Vault::new_empty(CONFIDENTIAL_TARI_RESOURCE_ADDRESS);
            Self {
                vault: Some(vault),
                ..Default::default()
            }
        }

        pub fn ref_stolen_vault(vault_id: tari_template_lib::models::VaultId) -> Self {
            Self {
                vault_ref: Some(vault_id),
                ..Default::default()
            }
        }

        pub fn with_stolen_vault(vault_id: tari_template_lib::models::VaultId) -> Component<Self> {
            let mut stolen = Vault::for_test(vault_id);
            Component::new(Self {
                vault: Some(stolen),
                ..Default::default()
            })
            .with_access_rules(AccessRules::allow_all())
            .with_owner_rule(OwnerRule::ByAccessRule(AccessRule::AllowAll))
            .create()
        }

        pub fn attempt_to_steal_funds_using_cross_template_call(
            vault_id: tari_template_lib::models::VaultId,
            dest_component: ComponentAddress,
            amount: Option<Amount>,
        ) {
            debug!("Attempting to steal funds from vault {}", vault_id);
            let mut vault = Vault::for_test(vault_id);
            let stolen = if let Some(amt) = amount {
                vault.withdraw(amt)
            } else {
                vault.withdraw_all()
            };
            ComponentManager::get(dest_component).call("deposit", args![stolen])
        }

        pub fn with_vault_copy() -> Self {
            let vault = Vault::new_empty(CONFIDENTIAL_TARI_RESOURCE_ADDRESS);
            let vault_copy = Vault::for_test(vault.vault_id());
            Self {
                vault: Some(vault),
                vault_copy: Some(vault_copy),
                ..Default::default()
            }
        }

        pub fn dangling_resource() -> Self {
            let _resx = ResourceBuilder::non_fungible().build();
            Self::default()
        }

        pub fn dangling_component() {
            let _component = Component::new(Self::default()).create();
        }

        pub fn dangling_component2() -> Self {
            let resx = ResourceBuilder::non_fungible().build();
            let _component = Component::new(Self {
                resource_address: Some(resx),
                ..Default::default()
            })
            .create();

            Self::default()
        }

        pub fn nested_component() -> Self {
            let resx = ResourceBuilder::non_fungible().build();
            let component = Component::new(Self {
                resource_address: Some(resx),
                ..Default::default()
            })
            .create();

            Self {
                component_address: Some(*component.address()),
                ..Default::default()
            }
        }

        pub fn non_existent_id() -> Self {
            Self {
                resource_address: Some(ResourceAddress::from([0xabu8; 28])),
                ..Default::default()
            }
        }

        pub fn clear(&mut self) {
            *self = Self::default();
        }

        pub fn drop_vault(&mut self) {
            self.vault = None;
        }

        pub fn take_bucket_zero(&mut self) {
            // Take a guess that there is a bucket with id == 0
            let stolen_bucket = Bucket::from_id(0u32.into());
            self.vault.as_mut().unwrap().deposit(stolen_bucket);
        }

        pub fn use_proof_zero(&mut self) {
            // Take a guess that there is a proof with id == 0
            let stolen_proof = Proof::from_id(0u32.into());
            let _auth = stolen_proof.authorize();
        }

        pub fn take_from_a_vault(&mut self, vault_id: tari_template_lib::models::VaultId, amount: Amount) {
            let mut vault = Vault::for_test(vault_id);
            let stolen = vault.withdraw(amount);
            self.vault.as_mut().unwrap().deposit(stolen);
        }

        pub fn empty_state_on_component(&self, address: tari_template_lib::models::ComponentAddress) {
            ComponentManager::get(address).set_state(());
        }

        pub fn deposit(&mut self, bucket: Bucket) {
            self.vault.as_mut().unwrap().deposit(bucket);
        }
    }
}
