//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::*;

const BADGE_NAMES: [&str; 4] = ["mint", "burn", "withdraw", "deposit"];

pub fn create_badge_resource(recall_rule: AccessRule) -> Bucket {
    let mut metadata = Metadata::new();
    metadata.insert("colour", "blue");
    ResourceBuilder::non_fungible()
        .recallable(recall_rule, OWNER)
        .initial_supply_with_data(
            BADGE_NAMES
                .into_iter()
                .map(|name| (NonFungibleId::from_string(name), (&metadata, &()))),
        )
}

#[template]
mod access_rules_template {
    use tari_template_lib::types::FunctionName;

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
            let tokens = ResourceBuilder::public_fungible()
                .with_owner_rule(owner_rule.clone())
                .with_access_rules(resource_rules)
                .initial_supply(1000u32);

            let badges = create_badge_resource(recall_rule);
            info!("Badges resource address: {}", badges.resource_address());

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

            let tokens = ResourceBuilder::public_fungible().initial_supply(1000u32);

            Component::create(Self {
                value: 0,
                tokens: Vault::from_bucket(tokens),
                badges: Vault::from_bucket(badges),
                allowed: true,
                attack_component: None,
            })
        }

        pub fn with_auth_hook(allowed: bool, hook: FunctionName) -> Component<AccessRulesTest> {
            let badges = create_badge_resource(rule!(deny_all));

            let address_alloc = CallerContext::allocate_component_address(None);

            let tokens = ResourceBuilder::public_fungible()
                .with_authorization_hook(address_alloc.get_address(), hook)
                .initial_supply(1000u32);

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

        /// As `with_auth_hook`, but anyone may mint and burn the tokens, so the hook can be exercised on the
        /// resource actions that take the resource's write lock.
        pub fn with_mintable_auth_hook(hook: FunctionName) -> Component<AccessRulesTest> {
            let badges = create_badge_resource(rule!(deny_all));

            let address_alloc = CallerContext::allocate_component_address(None);

            let tokens = ResourceBuilder::public_fungible()
                .with_authorization_hook(address_alloc.get_address(), hook)
                .mintable(rule!(allow_all), LOCKED)
                .burnable(rule!(allow_all), LOCKED)
                .initial_supply(1000u32);

            Component::new(Self {
                value: 0,
                tokens: Vault::from_bucket(tokens),
                badges: Vault::from_bucket(badges),
                allowed: true,
                attack_component: None,
            })
            .with_address_allocation(address_alloc)
            .with_access_rules(ComponentAccessRules::new().default(rule!(allow_all)))
            .create()
        }

        /// As `with_auth_hook`, but the hook may be replaced or removed by whoever satisfies `updater`.
        pub fn with_updatable_auth_hook(
            allowed: bool,
            hook: FunctionName,
            updater: UpdateRule,
        ) -> Component<AccessRulesTest> {
            let badges = create_badge_resource(rule!(deny_all));

            let address_alloc = CallerContext::allocate_component_address(None);

            let tokens = ResourceBuilder::public_fungible()
                .with_authorization_hook(address_alloc.get_address(), hook)
                .with_authorization_hook_updater(updater)
                .initial_supply(1000u32);

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
            let id = address_alloc.id();
            info!("Allocated address: {id}");

            let tokens = ResourceBuilder::public_fungible()
                .with_authorization_hook(
                    address_alloc.get_address(),
                    "malicious_auth_hook_set_state_on_another_component",
                )
                .initial_supply(1000u32);

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

        pub fn with_auth_hook_gated_on_caller(hook_caller: ComponentAddress) -> Component<AccessRulesTest> {
            let badges = create_badge_resource(rule!(deny_all));

            let address_alloc = CallerContext::allocate_component_address(None);

            let tokens = ResourceBuilder::public_fungible()
                .with_authorization_hook(address_alloc.get_address(), "caller_gated_hook")
                .initial_supply(1000u32);

            Component::new(Self {
                value: 0,
                tokens: Vault::from_bucket(tokens),
                badges: Vault::from_bucket(badges),
                allowed: true,
                attack_component: None,
            })
            .with_address_allocation(address_alloc)
            .with_owner_rule(OwnerRule::None)
            .with_access_rules(
                ComponentAccessRules::new()
                    .method("caller_gated_hook", rule!(caller_component(hook_caller)))
                    .method("take_tokens", rule!(allow_all))
                    .default(rule!(deny_all)),
            )
            .create()
        }

        pub fn using_badge_rules() -> Component<AccessRulesTest> {
            let badges = create_badge_resource(rule!(allow_all));

            let badge_resource = badges.resource_address();
            let tokens = ResourceBuilder::public_fungible()
                .mintable(
                    rule!(non_fungible(NonFungibleAddress::new(
                        badge_resource,
                        NonFungibleId::from_string("mint")
                    ))),
                    OWNER,
                )
                .burnable(
                    rule!(non_fungible(NonFungibleAddress::new(
                        badge_resource,
                        NonFungibleId::from_string("burn")
                    ))),
                    OWNER,
                )
                .withdrawable(
                    rule!(non_fungible(NonFungibleAddress::new(
                        badge_resource,
                        NonFungibleId::from_string("withdraw")
                    ))),
                    OWNER,
                )
                .depositable(
                    rule!(non_fungible(NonFungibleAddress::new(
                        badge_resource,
                        NonFungibleId::from_string("deposit")
                    ))),
                    OWNER,
                )
                .initial_supply(1000u32);

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
            let tokens = ResourceBuilder::public_fungible()
                .mintable(rule!(resource(badge_resource)), OWNER)
                .burnable(rule!(resource(badge_resource)), OWNER)
                .withdrawable(rule!(resource(badge_resource)), OWNER)
                .depositable(rule!(resource(badge_resource)), OWNER)
                .initial_supply(1000u32);

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
            let tokens = ResourceBuilder::public_fungible()
                .mintable(rule!(component(allocation.get_address())), LOCKED)
                // Only access rules apply, this just makes the test simpler because we do not need to change the transaction signer
                .with_owner_rule(OwnerRule::None)
                .initial_supply(1000u32);

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
            assert_eq!(
                *caller.resource(),
                self.tokens.resource_address(),
                "hook invoked for a resource this component does not manage"
            );
            let state = caller.component_state();
            debug!("Component state {:?}", state);
            if !self.allowed {
                panic!("Access denied for action {:?}", action);
            }
        }

        /// Auth hook whose method rule gates on the acting caller. Used to verify that the hook observes the
        /// acting component (not the hook author) as its caller.
        ///
        /// The caller gate alone is never sufficient: any resource may bind this method as its hook, so a
        /// gated caller acting on a hostile resource still passes the method rule. The hook must check the
        /// resource itself.
        pub fn caller_gated_hook(&self, _action: ResourceAuthAction, caller: AuthHookCaller) {
            assert_eq!(
                *caller.resource(),
                self.tokens.resource_address(),
                "hook invoked for a resource this component does not manage"
            );
        }

        pub fn malicious_auth_hook_set_state(&self, action: ResourceAuthAction, caller: AuthHookCaller) {
            debug!("malicious_auth_hook_set_state: action = {:?}", action);
            let caller = caller.component().unwrap();
            // Try to write component state - this should fail.
            // Typically, a transaction would have write access to the caller component. However, the caller component
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

        /// Points the managed resource's hook at `hook` on this component, or removes it when `hook` is None.
        pub fn set_auth_hook(&self, hook: Option<FunctionName>) {
            let hook = hook.map(|method| AuthHook::new(CallerContext::current_component_address(), method));
            ResourceManager::get(self.tokens.resource_address()).set_auth_hook(hook);
        }

        /// A mutable hook records each invocation in its own state, the one write a hook frame may make.
        pub fn counting_auth_hook(&mut self, _action: ResourceAuthAction, _caller: AuthHookCaller) {
            self.value += 1;
        }

        /// Attempts a state write outside the hook's own component. The hook frame is confined to its own
        /// component state, so the engine refuses the vault creation.
        pub fn hook_creates_vault(&self, _action: ResourceAuthAction, _caller: AuthHookCaller) {
            let _vault = Vault::new_empty(self.tokens.resource_address());
        }

        /// Reads the resource the hook guards. A resource action that write-locks the resource must release that
        /// lock across the hook call, or this read is refused.
        pub fn hook_reads_own_resource(&self, action: ResourceAuthAction, _caller: AuthHookCaller) {
            let manager = ResourceManager::get(self.tokens.resource_address());
            assert_eq!(
                manager.resource_type(),
                ResourceType::Fungible,
                "hook read the wrong resource for action {action:?}"
            );
        }

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

        pub fn update_tokens_access_rule(&mut self, action: ResourceAuthAction, new_rule: AccessRule) {
            ResourceManager::get(self.tokens.resource_address()).update_access_rule(action, new_rule);
        }

        pub fn set_tokens_metadata(&mut self, metadata: Metadata) {
            ResourceManager::get(self.tokens.resource_address()).set_metadata(metadata);
        }

        pub fn create_proof_from_bucket(bucket: Bucket) -> Proof {
            bucket.create_proof()
        }

        pub fn mint_resource(resource: ResourceAddress) -> Bucket {
            let manager = ResourceManager::get(resource);
            match manager.resource_type() {
                ResourceType::Fungible => manager.mint_fungible(1000u32),
                ResourceType::NonFungible => manager.mint_non_fungible(NonFungibleId::random(), &(), &()),
                ty => panic!("Unsupported resource type {:?}", ty),
            }
        }

        pub fn mint_more_tokens(&mut self, amount: Amount) -> Bucket {
            ResourceManager::get(self.tokens.resource_address()).mint_fungible(amount)
        }
    }
}
