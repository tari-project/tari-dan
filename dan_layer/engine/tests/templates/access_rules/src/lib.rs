//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::*;

const BADGE_NAMES: [&str; 4] = ["mint", "burn", "withdraw", "deposit"];

pub fn create_badge_resource(recall_rule: AccessRule) -> Bucket {
    let mut metadata = Metadata::new();
    metadata.insert("colour", "blue");
    ResourceBuilder::non_fungible()
        .recallable(recall_rule)
        .initial_supply_with_data(
            BADGE_NAMES
                .into_iter()
                .map(|name| (NonFungibleId::from_string(name), (&metadata, &()))),
        )
}

#[template]
mod access_rules_template {
    use super::*;

    pub struct AccessRulesTest {
        value: u32,
        tokens: Vault,
        badges: Vault,
        allowed: bool,
        attack_component: Option<ComponentAddress>,
    }

    impl AccessRulesTest {
        pub fn with_configured_rules(
            owner_rule: OwnerRule,
            component_access_rule: ComponentAccessRules,
            resource_rules: ResourceAccessRules,
            recall_rule: AccessRule,
        ) -> Component<AccessRulesTest> {
            let tokens = ResourceBuilder::fungible()
                .with_owner_rule(owner_rule.clone())
                .with_access_rules(resource_rules)
                .initial_supply(1000);

            let badges = create_badge_resource(recall_rule);

            Component::new(Self {
                value: 0,
                tokens: Vault::from_bucket(tokens),
                badges: Vault::from_bucket(badges),
                allowed: true,
                attack_component: None,
            })
            .with_owner_rule(owner_rule)
            .with_access_rules(component_access_rule)
            .create()
        }

        pub fn default_rules() -> Component<AccessRulesTest> {
            let badges = create_badge_resource(rule!(deny_all));

            let tokens = ResourceBuilder::fungible().initial_supply(1000);

            Component::create(Self {
                value: 0,
                tokens: Vault::from_bucket(tokens),
                badges: Vault::from_bucket(badges),
                allowed: true,
                attack_component: None,
            })
        }

        pub fn with_auth_hook(allowed: bool, hook: String) -> Component<AccessRulesTest> {
            let badges = create_badge_resource(rule!(deny_all));

            let address_alloc = CallerContext::allocate_component_address(None);

            let tokens = ResourceBuilder::fungible()
                .with_authorization_hook(*address_alloc.address(), hook)
                .initial_supply(1000);

            Component::new(Self {
                value: 0,
                tokens: Vault::from_bucket(tokens),
                badges: Vault::from_bucket(badges),
                allowed,
                attack_component: None,
            })
            .with_address_allocation(address_alloc)
            .with_access_rules(ComponentAccessRules::new().default(rule!(allow_all)))
            .create()
        }

        pub fn with_auth_hook_attack_component(component_address: ComponentAddress) -> Component<AccessRulesTest> {
            let badges = create_badge_resource(rule!(deny_all));

            let address_alloc = CallerContext::allocate_component_address(None);

            let tokens = ResourceBuilder::fungible()
                .with_authorization_hook(
                    *address_alloc.address(),
                    "malicious_auth_hook_set_state_on_another_component",
                )
                .initial_supply(1000);

            Component::new(Self {
                value: 0,
                tokens: Vault::from_bucket(tokens),
                badges: Vault::from_bucket(badges),
                allowed: true,
                attack_component: Some(component_address),
            })
            .with_address_allocation(address_alloc)
            .with_access_rules(ComponentAccessRules::new().default(rule!(allow_all)))
            .create()
        }

        pub fn using_badge_rules() -> Component<AccessRulesTest> {
            let badges = create_badge_resource(rule!(allow_all));

            let badge_resource = badges.resource_address();
            let tokens = ResourceBuilder::fungible()
                .mintable(rule!(non_fungible(
                    NonFungibleAddress::new(badge_resource, NonFungibleId::from_string("mint")))))
                .burnable(rule!(non_fungible(
                    NonFungibleAddress::new(badge_resource, NonFungibleId::from_string("burn")))))
                .withdrawable(rule!(non_fungible(
                    NonFungibleAddress::new(badge_resource, NonFungibleId::from_string("withdraw")))))
                .depositable(rule!(non_fungible(
                    NonFungibleAddress::new(badge_resource, NonFungibleId::from_string("deposit")))))
                .initial_supply(1000);

            Component::new(Self {
                value: 0,
                tokens: Vault::from_bucket(tokens),
                badges: Vault::from_bucket(badges),
                allowed: true,
                attack_component: None,
            })
            .with_access_rules(ComponentAccessRules::new().default(rule!(allow_all)))
            .create()
        }

        pub fn using_resource_rules() -> Component<AccessRulesTest> {
            let badges = create_badge_resource(rule!(allow_all));

            let badge_resource = badges.resource_address();
            let tokens = ResourceBuilder::fungible()
                .mintable(rule!(non_fungible(badge_resource)))
                .burnable(rule!(non_fungible(badge_resource)))
                .withdrawable(rule!(non_fungible(badge_resource)))
                .depositable(rule!(non_fungible(badge_resource)))
                .initial_supply(1000);

            Component::new(Self {
                value: 0,
                tokens: Vault::from_bucket(tokens),
                badges: Vault::from_bucket(badges),
                allowed: true,
                attack_component: None,
            })
            .with_access_rules(ComponentAccessRules::new().default(rule!(allow_all)))
            .create()
        }

        pub fn resource_actions_restricted_to_component() -> Component<AccessRulesTest> {
            let badges = create_badge_resource(rule!(allow_all));

            let allocation = CallerContext::allocate_component_address(None);
            let tokens = ResourceBuilder::fungible()
                .mintable(rule!(non_fungible(allocation.address())))
                // Only access rules apply, this just makes the test simpler because we do not need to change the transaction signer
                .with_owner_rule(OwnerRule::None)
                .initial_supply(1000);

            Component::new(Self {
                value: 0,
                tokens: Vault::from_bucket(tokens),
                badges: Vault::from_bucket(badges),
                allowed: true,
                attack_component: None,
            })
            .with_address_allocation(allocation)
            .with_access_rules(ComponentAccessRules::new().default(rule!(allow_all)))
            .create()
        }

        /// Custom resource auth hook
        pub fn valid_auth_hook(&self, action: ResourceAuthAction, caller: AuthHookCaller) {
            let state = caller.component_state();
            debug!("Component state {:?}", state);
            if !self.allowed {
                panic!("Access denied for action {:?}", action);
            }
        }

        pub fn malicious_auth_hook_set_state(&self, action: ResourceAuthAction, caller: AuthHookCaller) {
            debug!("malicious_auth_hook_set_state: action = {:?}", action);
            let caller = caller.component().unwrap();
            // Try to write component state - this should fail.
            // Typically a transaction would have write access to the caller component. However the caller component
            // will always have at least a read lock during the hook call, preventing this from working.

            ComponentManager::get(*caller).set_state(&());
        }

        pub fn malicious_auth_hook_call_mut(&self, action: ResourceAuthAction, caller: AuthHookCaller) {
            debug!("malicious_auth_hook_call_mut: action = {:?}", action);
            let caller = caller.component().unwrap();
            // Try to cross template call to a component - this should fail.
            let bucket = ComponentManager::get(*caller).call("withdraw", args![self.tokens.resource_address()]);
            self.tokens.deposit(bucket);
        }

        pub fn malicious_auth_hook_set_state_on_another_component(
            &self,
            action: ResourceAuthAction,
            _caller: AuthHookCaller,
        ) {
            debug!(
                "malicious_auth_hook_set_state_on_another_component: action = {:?}",
                action
            );
            // Try to cross template call to another component. This will succeed if the component allows all access to
            // the method, otherwise it will fail. Since the auth hook does not allow foreign proofs, there is no way to
            // authorize a restricted cross template call. We're really checking the semantics of cross-template calls,
            // not the auth hook.
            ComponentManager::get(self.attack_component.unwrap()).invoke("set", args![123]);
        }

        pub fn invalid_auth_hook1(&mut self, _action: ResourceAuthAction, _caller: AuthHookCaller) {}

        pub fn invalid_auth_hook2(&self, _action: String, _caller: AuthHookCaller) {}

        pub fn invalid_auth_hook3(&self, _action: ResourceAuthAction, _caller: String) {}

        pub fn invalid_auth_hook4(&self, _action: ResourceAuthAction, _caller: AuthHookCaller, _third: String) {}

        pub fn invalid_auth_hook5(&self, _action: ResourceAuthAction, _caller: AuthHookCaller) -> String {
            unimplemented!()
        }

        pub fn take_badge_by_name(&mut self, name: String) -> Bucket {
            self.badges.withdraw_non_fungible(NonFungibleId::from_string(&name))
        }

        pub fn recall_badge(&mut self, vault_id: VaultId, name: String) {
            let bucket = ResourceManager::get(self.badges.resource_address())
                .recall_non_fungible(vault_id, NonFungibleId::from_string(&name));
            self.badges.deposit(bucket)
        }

        pub fn mint_new_badge(&self) -> Bucket {
            let mut metadata = Metadata::new();
            metadata.insert("colour", "blue");
            ResourceManager::get(self.badges.resource_address()).mint_non_fungible(
                NonFungibleId::random(),
                &metadata,
                &(),
            )
        }

        pub fn take_tokens(&mut self, amount: Amount) -> Bucket {
            self.tokens.withdraw(amount)
        }

        pub fn take_tokens_using_proof(&mut self, proof: Proof, amount: Amount) -> Bucket {
            // let _access = proof.authorize(); is better if you want to panic immediately
            // try_authorize can be used to determine if access is permitted and still run some other code branch if
            // not.
            match proof.try_authorize() {
                Ok(_access) => self.tokens.withdraw(amount),
                Err(_) => {
                    debug!("Sorry, not allowed to take tokens");
                    panic!("Access denied");
                },
            }
        }

        pub fn get_nft_data_using_proof(&self, proof: Proof) -> Vec<Metadata> {
            let nfts = proof.get_non_fungibles();
            let manager = ResourceManager::get(proof.resource_address());
            nfts.iter()
                .map(|nft| manager.get_non_fungible(nft))
                .map(|nft| nft.get_data())
                .collect()
        }

        pub fn set_value(&mut self, value: u32) {
            debug!("Changing value from {} to {}", self.value, value);
            self.value = value;
        }

        pub fn get_value(&self) -> u32 {
            self.value
        }

        pub fn set_component_access_rules(&mut self, access_rules: ComponentAccessRules) {
            let component_addr = CallerContext::current_component_address();
            ComponentManager::get(component_addr).set_access_rules(access_rules);
        }

        pub fn set_tokens_access_rules(&mut self, access_rules: ResourceAccessRules) {
            ResourceManager::get(self.tokens.resource_address()).set_access_rules(access_rules);
        }

        pub fn create_proof_from_bucket(bucket: Bucket) -> Proof {
            bucket.create_proof()
        }

        pub fn mint_resource(resource: ResourceAddress) -> Bucket {
            let manager = ResourceManager::get(resource);
            match manager.resource_type() {
                ResourceType::Fungible => manager.mint_fungible(1000.into()),
                ResourceType::NonFungible => manager.mint_non_fungible(NonFungibleId::random(), &(), &()),
                ty => panic!("Unsupported resource type {:?}", ty),
            }
        }

        pub fn mint_more_tokens(&mut self, amount: Amount) -> Bucket {
            ResourceManager::get(self.tokens.resource_address()).mint_fungible(amount)
        }
    }
}
