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
        vault_ref: Option<VaultId>,
    }

    impl Shenanigans {
        pub fn dangling_vault() -> Self {
            let _vault = Vault::new_empty(STEALTH_TARI_RESOURCE_ADDRESS);
            Self::default()
        }

        pub fn return_vault() -> Vault {
            Vault::new_empty(STEALTH_TARI_RESOURCE_ADDRESS)
        }

        pub fn new() -> Self {
            Self::default()
        }

        pub fn new_with_address(addr: ComponentAddressAllocation) -> Component<Self> {
            Component::new(Self::default()).with_address_allocation(addr).create()
        }

        pub fn with_vault() -> Self {
            let vault = Vault::new_empty(STEALTH_TARI_RESOURCE_ADDRESS);
            Self {
                vault: Some(vault),
                ..Default::default()
            }
        }

        pub fn ref_stolen_vault(vault_id: VaultId) -> Self {
            Self {
                vault_ref: Some(vault_id.into()),
                ..Default::default()
            }
        }

        pub fn with_stolen_vault(vault_id: VaultId) -> Component<Self> {
            let stolen = Vault::for_test(vault_id.into());
            Component::new(Self {
                vault: Some(stolen),
                ..Default::default()
            })
            .with_access_rules(AccessRules::allow_all())
            .with_owner_rule(OwnerRule::ByAccessRule(rule!(allow_all)))
            .create()
        }

        pub fn attempt_to_steal_funds_using_cross_template_call(
            vault_id: VaultId,
            dest_component: ComponentAddress,
            amount: Option<Amount>,
        ) {
            debug!("Attempting to steal funds from vault {}", vault_id);
            let mut vault = Vault::for_test(vault_id.into());
            let stolen = if let Some(amt) = amount {
                vault.withdraw(amt)
            } else {
                vault.withdraw_all()
            };
            ComponentManager::get(dest_component).call("deposit", args![stolen])
        }

        pub fn with_vault_copy() -> Self {
            let vault = Vault::new_empty(STEALTH_TARI_RESOURCE_ADDRESS);
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
                resource_address: Some(ResourceAddress::from([0xabu8; 32])),
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

        pub fn take_from_a_vault(&mut self, vault_id: VaultId, amount: Amount) {
            let mut vault = Vault::for_test(vault_id.into());
            let stolen = vault.withdraw(amount);
            self.vault.as_mut().unwrap().deposit(stolen);
        }

        pub fn take_from_vault_and_return_bucket(vault_id: VaultId) -> Bucket {
            let mut stolen = Vault::for_test(vault_id.into());
            stolen.withdraw_all()
        }

        pub fn take_from_hardcoded_vault() -> Bucket {
            let vault_id = option_env!["VAULT_ID"].expect("VAULT_ID must be set at compile time");
            let mut stolen = Vault::for_test(vault_id.parse().unwrap());
            stolen.withdraw_all()
        }

        pub fn take_from_hardcoded_vault_in_component_context(&self) -> Bucket {
            let vault_id = option_env!["VAULT_ID"].expect("VAULT_ID must be set at compile time");
            let mut stolen = Vault::for_test(vault_id.parse().unwrap());
            stolen.withdraw_all()
        }

        /// Calls a method the victim permits, then withdraws directly from a vault of the victim's it never
        /// handed over.
        pub fn call_then_steal_from_vault(victim: ComponentAddress, vault_id: VaultId) -> Bucket {
            let _balances: Vec<(ResourceAddress, Amount)> =
                ComponentManager::get(victim).call("get_balances", args![]);
            let mut stolen = Vault::for_test(vault_id.into());
            stolen.withdraw_all()
        }

        pub fn with_fungible_vault() -> Component<Self> {
            let tokens = ResourceBuilder::public_fungible().initial_supply(1000u32);
            Component::new(Self {
                vault: Some(Vault::from_bucket(tokens)),
                ..Default::default()
            })
            .with_access_rules(AccessRules::allow_all())
            .with_owner_rule(OwnerRule::ByAccessRule(rule!(allow_all)))
            .create()
        }

        pub fn abandon_bucket(&mut self) {
            let _bucket = self.vault.as_mut().unwrap().withdraw(Amount::from(1u64));
        }

        /// A `ProofAccess` guard leaves the auth scope when it is dropped; the proof it came from is still held and
        /// may be dropped or returned afterwards.
        pub fn authorize_then_drop_proof(&self) {
            let proof = self.vault.as_ref().unwrap().create_proof();
            {
                let _auth = proof.authorize();
            }
            proof.drop();
        }

        pub fn authorize_then_return_proof(&self) -> Proof {
            let proof = self.vault.as_ref().unwrap().create_proof();
            {
                let _auth = proof.authorize();
            }
            proof
        }

        pub fn abandon_proof(&mut self) {
            let _proof = self.vault.as_ref().unwrap().create_proof();
        }

        pub fn empty_state_on_component(&self, address: ComponentAddress) {
            ComponentManager::get(address).set_state(());
        }

        pub fn deposit(&mut self, bucket: Bucket) {
            self.vault.as_mut().unwrap().deposit(bucket);
        }
    }
}
