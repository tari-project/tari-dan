//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::*;

pub fn create_badge_resource(recall_rule: AccessRule) -> Bucket {
    ResourceBuilder::non_fungible()
        .with_non_fungible(NonFungibleId::from_string("mint"), &(), &())
        .with_non_fungible(NonFungibleId::from_string("burn"), &(), &())
        .with_non_fungible(NonFungibleId::from_string("withdraw"), &(), &())
        .with_non_fungible(NonFungibleId::from_string("deposit"), &(), &())
        .recallable(recall_rule)
        .build_bucket()
}

#[template]
mod access_rules_template {
    use super::*;

    pub struct AccessRulesTest {
        value: u32,
        tokens: Vault,
        badges: Vault,
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
                .initial_supply(1000)
                .build_bucket();

            let badges = create_badge_resource(recall_rule);

            Component::new(Self {
                value: 0,
                tokens: Vault::from_bucket(tokens),
                badges: Vault::from_bucket(badges),
            })
            .with_owner_rule(owner_rule)
            .with_access_rules(component_access_rule)
            .create()
        }

        pub fn default_rules() -> Component<AccessRulesTest> {
            let badges = create_badge_resource(AccessRule::DenyAll);

            let tokens = ResourceBuilder::fungible().initial_supply(1000).build_bucket();

            Component::create(Self {
                value: 0,
                tokens: Vault::from_bucket(tokens),
                badges: Vault::from_bucket(badges),
            })
        }

        pub fn using_badge_rules() -> Component<AccessRulesTest> {
            let badges = create_badge_resource(AccessRule::AllowAll);

            let badge_resource = badges.resource_address();
            let tokens = ResourceBuilder::fungible()
                .initial_supply(1000)
                .mintable(AccessRule::Restricted(RestrictedAccessRule::Require(
                    RequireRule::Require(
                        NonFungibleAddress::new(badge_resource, NonFungibleId::from_string("mint")).into(),
                    ),
                )))
                .burnable(AccessRule::Restricted(RestrictedAccessRule::Require(
                    RequireRule::Require(
                        NonFungibleAddress::new(badge_resource, NonFungibleId::from_string("burn")).into(),
                    ),
                )))
                .withdrawable(AccessRule::Restricted(RestrictedAccessRule::Require(
                    RequireRule::Require(
                        NonFungibleAddress::new(badge_resource, NonFungibleId::from_string("withdraw")).into(),
                    ),
                )))
                .depositable(AccessRule::Restricted(RestrictedAccessRule::Require(
                    RequireRule::Require(
                        NonFungibleAddress::new(badge_resource, NonFungibleId::from_string("deposit")).into(),
                    ),
                )))
                .build_bucket();

            Component::new(Self {
                value: 0,
                tokens: Vault::from_bucket(tokens),
                badges: Vault::from_bucket(badges),
            })
            .with_access_rules(ComponentAccessRules::new().default(AccessRule::AllowAll))
            .create()
        }

        pub fn using_resource_rules() -> Component<AccessRulesTest> {
            let badges = create_badge_resource(AccessRule::AllowAll);

            let badge_resource = badges.resource_address();
            let tokens = ResourceBuilder::fungible()
                .initial_supply(1000)
                .mintable(AccessRule::Restricted(RestrictedAccessRule::Require(
                    RequireRule::Require(badge_resource.into()),
                )))
                .burnable(AccessRule::Restricted(RestrictedAccessRule::Require(
                    RequireRule::Require(badge_resource.into()),
                )))
                .withdrawable(AccessRule::Restricted(RestrictedAccessRule::Require(
                    RequireRule::Require(badge_resource.into()),
                )))
                .depositable(AccessRule::Restricted(RestrictedAccessRule::Require(
                    RequireRule::Require(badge_resource.into()),
                )))
                .build_bucket();

            Component::new(Self {
                value: 0,
                tokens: Vault::from_bucket(tokens),
                badges: Vault::from_bucket(badges),
            })
            .with_access_rules(ComponentAccessRules::new().default(AccessRule::AllowAll))
            .create()
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
            ResourceManager::get(self.badges.resource_address()).mint_non_fungible(NonFungibleId::random(), &(), &())
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
    }
}
