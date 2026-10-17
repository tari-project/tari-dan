//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use std::collections::{BTreeMap, HashMap};

use tari_engine::runtime::{ActionIdent, RuntimeError};
use tari_engine_types::resource_container::ResourceError;
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::{
    args::ComponentAction,
    types::{
        AccessRule,
        ComponentAddress,
        Metadata,
        NonFungibleId,
        OwnerRule,
        ResourceAddress,
        VaultId,
        access_rules::{
            ComponentAccessRules,
            OWNER,
            RequireRule,
            ResourceAccessRules,
            ResourceAuthAction,
            RestrictedAccessRule,
            RuleRequirement,
            UpdateRule,
        },
        constants::TARI_TOKEN,
        rule,
    },
};
use tari_template_test_tooling::{
    TemplateTest,
    support::assert_error::{
        assert_access_denied_for_action,
        assert_insufficient_funds_for_action,
        assert_reject_reason,
    },
};

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

mod component_access_rules {
    use super::*;

    #[test]
    fn it_restricts_component_methods() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        // Create sender and receiver accounts
        let (owner1_proof, _, owner1_key) = test.create_owner_proof();
        let (owner2_proof, _, owner2_key) = test.create_owner_proof();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        let owner_rule = AccessRule::Restricted(
            RestrictedAccessRule::Require(RequireRule::Require(owner1_proof.clone().into())).or(
                RestrictedAccessRule::Require(RequireRule::Require(owner2_proof.clone().into())),
            ),
        );

        let component_rules = ComponentAccessRules::new()
            .add_method_rule("set_value", owner_rule.clone())
            .default(AccessRule::DenyAll);

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_configured_rules", args![
                    // Owner
                    OwnerRule::ByAccessRule(owner_rule),
                    // Component
                    component_rules,
                    // Resource
                    ResourceAccessRules::deny_all(),
                    // Badge recall rule
                    AccessRule::DenyAll,
                ])
                .build_and_seal(&owner1_key),
            // Because we deny_all on deposits, we need to supply the owner proof to be able to deposit the initial
            // tokens into the new vaults
            vec![owner1_proof.clone()],
        );

        let (component_address, _) = test
            .read_only_state_store()
            .get_components_by_template_address(access_rules_template)
            .unwrap()
            .first()
            .unwrap()
            .clone();

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "set_value", args![1])
                .build_and_seal(&owner2_key),
            vec![owner2_proof],
        );

        let (unauth_proof, _, unauth_key) = test.create_owner_proof();

        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "set_value", args![1])
                .build_and_seal(&unauth_key),
            vec![unauth_proof],
        );

        assert_access_denied_for_action(reason, ActionIdent::ComponentCallMethod {
            component_address,
            method: "set_value".to_string(),
        });
    }

    #[test]
    fn it_allows_owner_to_update_component_access_rules() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        // Create sender and receiver accounts
        let (owner_proof, _, owner_key) = test.create_owner_proof();
        let (user_proof, _, user_key) = test.create_owner_proof();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_configured_rules", args![
                    // Owner
                    OwnerRule::OwnedBySigner,
                    // Component
                    ComponentAccessRules::new().default(AccessRule::DenyAll),
                    // Resource
                    ResourceAccessRules::deny_all(),
                    // Badge recall rule
                    AccessRule::DenyAll
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        // Access Denied
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "set_value", args![1])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        assert_access_denied_for_action(reason, ActionIdent::ComponentCallMethod {
            component_address,
            method: "set_value".to_string(),
        });

        // Allow user to call set_value
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "set_component_access_rules", args![
                    ComponentAccessRules::new()
                        .add_method_rule("set_value", rule!(non_fungible(user_proof.clone())))
                        .default(AccessRule::DenyAll)
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "set_value", args![1])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "set_component_access_rules", args![
                    ComponentAccessRules::new().default(AccessRule::AllowAll)
                ])
                .build_and_seal(&user_key),
            vec![user_proof],
        );
    }

    #[test]
    fn it_prevents_access_rule_modification_if_owner_is_none() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        // Create sender and receiver accounts
        let (owner_proof, _, owner_key) = test.create_owner_proof();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_configured_rules", args![
                    // Owner
                    OwnerRule::None,
                    // Component
                    ComponentAccessRules::new().default(AccessRule::AllowAll),
                    // Resource
                    ResourceAccessRules::new(),
                    // Badge recall rule
                    AccessRule::DenyAll,
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        // Owner cannot set access rules
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "set_component_access_rules", args![
                    ComponentAccessRules::new().default(AccessRule::AllowAll)
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );

        assert_reject_reason(reason, RuntimeError::AccessDeniedOwnerRequired {
            action: ComponentAction::SetAccessRules.into(),
        });
    }

    #[test]
    fn set_access_rules_rejects_scoped_requirement() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (owner_proof, _, owner_key) = test.create_owner_proof();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_configured_rules", args![
                    OwnerRule::OwnedBySigner,
                    ComponentAccessRules::new().default(AccessRule::AllowAll),
                    ResourceAccessRules::new(),
                    AccessRule::DenyAll,
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        // Build a rule set that bypasses the builder lint, the way a hand-written or non-Rust template
        // could, and confirm the engine rejects it rather than installing a constant method rule.
        let degenerate = component_access_rules_with_scoped_method_rule(component_address);
        assert!(degenerate.contains_scoped_to_component_or_template());

        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "set_component_access_rules", args![degenerate])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );

        assert_reject_reason(reason, RuntimeError::InvalidArgument {
            argument: "access_rules",
            reason: "component(..)/template(..) cannot be used on a component method access rule".to_string(),
        });
    }

    #[test]
    fn create_component_rejects_scoped_owner_rule() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (owner_proof, _, owner_key) = test.create_owner_proof();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_configured_rules", args![
                    OwnerRule::OwnedBySigner,
                    ComponentAccessRules::new().default(AccessRule::AllowAll),
                    ResourceAccessRules::new(),
                    AccessRule::DenyAll,
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );
        let some_component = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        // A `component(..)` owner rule is constant on a component (its own frame is always on top), so the
        // engine rejects it at creation rather than installing an "owned by everyone" rule.
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_configured_rules", args![
                    OwnerRule::ByAccessRule(rule!(component(some_component))),
                    ComponentAccessRules::new().default(AccessRule::AllowAll),
                    ResourceAccessRules::new(),
                    AccessRule::DenyAll,
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );

        assert_reject_reason(reason, RuntimeError::InvalidArgument {
            argument: "owner_rule",
            reason: "component(..)/template(..) cannot be used in a component owner rule".to_string(),
        });
    }

    fn component_access_rules_with_scoped_method_rule(component: ComponentAddress) -> ComponentAccessRules {
        use tari_bor::minicbor::{Encode, Encoder};

        // Build the degenerate rule through the public enums and encode/decode it as a full
        // `ComponentAccessRules`, bypassing the builder lint the way a hand-written or non-Rust template
        // would.
        let scoped_rule = AccessRule::Restricted(RestrictedAccessRule::Require(RequireRule::Require(
            RuleRequirement::ScopedToComponent(component),
        )));

        let mut e = Encoder::new(Vec::new());
        // ComponentAccessRules = [ method_access, default ] (positional struct array)
        e.array(2).unwrap();
        // method_access = { "set_value": scoped_rule }
        e.map(1).unwrap();
        e.str("set_value").unwrap();
        Encode::encode(&scoped_rule, &mut e, &mut ()).unwrap();
        // default = DenyAll
        Encode::encode(&AccessRule::DenyAll, &mut e, &mut ()).unwrap();

        let bytes = e.into_writer();
        tari_bor::decode(&bytes).unwrap()
    }
}

mod component_owner_rule {
    use ootle_byte_type::ToByteType;
    use tari_crypto::ristretto::{RistrettoPublicKey, RistrettoSecretKey};
    use tari_engine_types::{commit_result::RejectReason, substate::SubstateId};
    use tari_ootle_common_types::SubstateVersion;
    use tari_template_lib::types::{NonFungibleAddress, SubstateOwnerRule};

    use super::*;

    struct Owner {
        proof: NonFungibleAddress,
        public_key: RistrettoPublicKey,
        secret_key: RistrettoSecretKey,
    }

    impl Owner {
        fn new(test: &mut TemplateTest) -> Self {
            let (proof, public_key, secret_key) = test.create_owner_proof();
            Self {
                proof,
                public_key,
                secret_key,
            }
        }

        fn as_owner_rule(&self) -> SubstateOwnerRule {
            SubstateOwnerRule::ByPublicKey(self.public_key.to_byte_type())
        }
    }

    /// Creates a component owned by `owner` whose methods anyone may call, so only the owner rule gates the
    /// component actions under test.
    fn create_component(test: &mut TemplateTest, owner: &Owner) -> ComponentAddress {
        let template = test.get_template_address("AccessRulesTest");
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(template, "with_configured_rules", args![
                    OwnerRule::OwnedBySigner,
                    ComponentAccessRules::new().default(AccessRule::AllowAll),
                    ResourceAccessRules::new(),
                    AccessRule::DenyAll,
                ])
                .build_and_seal(&owner.secret_key),
            vec![owner.proof.clone()],
        );
        test.return_values()[0].decoded::<ComponentAddress>().unwrap()
    }

    fn set_owner_rule(caller: &Owner, component: ComponentAddress, owner_rule: SubstateOwnerRule) -> Transaction {
        Transaction::builder_localnet(Epoch(1))
            .call_method(component, "set_component_owner_rule", args![owner_rule])
            .build_and_seal(&caller.secret_key)
    }

    fn set_access_rules(caller: &Owner, component: ComponentAddress) -> Transaction {
        Transaction::builder_localnet(Epoch(1))
            .call_method(component, "set_component_access_rules", args![
                ComponentAccessRules::new().default(AccessRule::AllowAll)
            ])
            .build_and_seal(&caller.secret_key)
    }

    fn succeeds(test: &mut TemplateTest, caller: &Owner, transaction: Transaction) {
        test.execute_expect_success(transaction, vec![caller.proof.clone()]);
    }

    fn fails(test: &mut TemplateTest, caller: &Owner, transaction: Transaction) -> RejectReason {
        test.execute_expect_failure(transaction, vec![caller.proof.clone()])
    }

    fn owner_rule_of(test: &TemplateTest, component: ComponentAddress) -> SubstateOwnerRule {
        test.read_only_state_store()
            .get_substate(&SubstateId::Component(component))
            .unwrap()
            .substate_value()
            .component()
            .unwrap()
            .owner_rule()
            .clone()
    }

    fn version_of(test: &TemplateTest, component: ComponentAddress) -> SubstateVersion {
        test.read_only_state_store()
            .get_substate(&SubstateId::Component(component))
            .unwrap()
            .version()
    }

    fn owner_required() -> RuntimeError {
        RuntimeError::AccessDeniedOwnerRequired {
            action: ComponentAction::SetOwnerRule.into(),
        }
    }

    #[test]
    fn the_owner_can_set_the_owner_rule_and_a_non_owner_cannot() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);
        let owner = Owner::new(&mut test);
        let stranger = Owner::new(&mut test);
        let component = create_component(&mut test, &owner);

        let reason = fails(
            &mut test,
            &stranger,
            set_owner_rule(&stranger, component, stranger.as_owner_rule()),
        );
        assert_reject_reason(reason, owner_required());
        assert_eq!(owner_rule_of(&test, component), owner.as_owner_rule());

        let council = SubstateOwnerRule::ByAccessRule(rule!(any_of(
            non_fungible(owner.proof.clone()),
            non_fungible(stranger.proof.clone())
        )));
        succeeds(&mut test, &owner, set_owner_rule(&owner, component, council.clone()));
        assert_eq!(owner_rule_of(&test, component), council);
    }

    #[test]
    fn handing_over_ownership_locks_out_the_previous_owner() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);
        let outgoing = Owner::new(&mut test);
        let incoming = Owner::new(&mut test);
        let component = create_component(&mut test, &outgoing);

        succeeds(
            &mut test,
            &outgoing,
            set_owner_rule(&outgoing, component, incoming.as_owner_rule()),
        );

        let reason = fails(&mut test, &outgoing, set_access_rules(&outgoing, component));
        assert_reject_reason(reason, RuntimeError::AccessDeniedOwnerRequired {
            action: ComponentAction::SetAccessRules.into(),
        });
        let reason = fails(
            &mut test,
            &outgoing,
            set_owner_rule(&outgoing, component, outgoing.as_owner_rule()),
        );
        assert_reject_reason(reason, owner_required());

        succeeds(&mut test, &incoming, set_access_rules(&incoming, component));
        succeeds(
            &mut test,
            &incoming,
            set_owner_rule(&incoming, component, outgoing.as_owner_rule()),
        );
        assert_eq!(owner_rule_of(&test, component), outgoing.as_owner_rule());
    }

    #[test]
    fn an_owner_rule_of_none_is_final() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);
        let owner = Owner::new(&mut test);
        let component = create_component(&mut test, &owner);

        succeeds(
            &mut test,
            &owner,
            set_owner_rule(&owner, component, SubstateOwnerRule::None),
        );

        let reason = fails(
            &mut test,
            &owner,
            set_owner_rule(&owner, component, owner.as_owner_rule()),
        );
        assert_reject_reason(reason, owner_required());
        let reason = fails(&mut test, &owner, set_access_rules(&owner, component));
        assert_reject_reason(reason, RuntimeError::AccessDeniedOwnerRequired {
            action: ComponentAction::SetAccessRules.into(),
        });
        assert_eq!(owner_rule_of(&test, component), SubstateOwnerRule::None);
    }

    #[test]
    fn get_owner_proof_sees_an_owner_rule_set_earlier_in_the_transaction() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);
        let outgoing = Owner::new(&mut test);
        let incoming = Owner::new(&mut test);
        let component = create_component(&mut test, &outgoing);

        let reason = fails(
            &mut test,
            &outgoing,
            Transaction::builder_localnet(Epoch(1))
                .call_method(component, "set_component_owner_rule_then_get_owner_proof", args![
                    incoming.as_owner_rule()
                ])
                .build_and_seal(&outgoing.secret_key),
        );
        assert_reject_reason(reason, RuntimeError::SignerBadgeNotInScope {
            public_key: incoming.public_key.to_byte_type(),
        });
    }

    #[test]
    fn setting_the_current_owner_rule_leaves_the_component_unchanged() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);
        let owner = Owner::new(&mut test);
        let component = create_component(&mut test, &owner);
        let version = version_of(&test, component);

        succeeds(
            &mut test,
            &owner,
            set_owner_rule(&owner, component, owner.as_owner_rule()),
        );

        assert_eq!(version_of(&test, component), version);
    }

    #[test]
    fn an_owner_rule_scoped_to_a_component_is_rejected() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);
        let owner = Owner::new(&mut test);
        let component = create_component(&mut test, &owner);

        let scoped = SubstateOwnerRule::ByAccessRule(AccessRule::Restricted(RestrictedAccessRule::Require(
            RequireRule::Require(RuleRequirement::ScopedToComponent(component)),
        )));
        let reason = fails(&mut test, &owner, set_owner_rule(&owner, component, scoped));
        assert_reject_reason(reason, RuntimeError::InvalidArgument {
            argument: "owner_rule",
            reason: "component(..)/template(..) cannot be used in a component owner rule".to_string(),
        });
        assert_eq!(owner_rule_of(&test, component), owner.as_owner_rule());
    }
}

mod m_of_n_threshold {
    use tari_template_lib::types::{NonFungibleAddress, SubstateOwnerRule, crypto::RistrettoPublicKeyBytes};

    use super::*;

    fn invalid_threshold(argument: &'static str, threshold: u16, num_requirements: usize) -> RuntimeError {
        RuntimeError::InvalidMOfNThreshold {
            argument,
            threshold,
            num_requirements,
        }
    }

    fn signer_badge(test: &TemplateTest) -> NonFungibleAddress {
        NonFungibleAddress::from_public_key(test.to_public_key_bytes())
    }

    fn zero_of_one(badge: &NonFungibleAddress) -> AccessRule {
        rule!(m_of_n(0, non_fungible(badge.clone())))
    }

    fn two_of_one(badge: &NonFungibleAddress) -> AccessRule {
        rule!(m_of_n(2, non_fungible(badge.clone())))
    }

    fn create_component(
        test: &TemplateTest,
        owner_rule: OwnerRule,
        component_rules: ComponentAccessRules,
        resource_rules: ResourceAccessRules,
    ) -> Transaction {
        Transaction::builder_localnet(Epoch(1))
            .call_function(
                test.get_template_address("AccessRulesTest"),
                "with_configured_rules",
                args![owner_rule, component_rules, resource_rules, AccessRule::DenyAll,],
            )
            .build_and_seal(test.secret_key())
    }

    #[test]
    fn creating_a_component_or_resource_with_an_invalid_threshold_is_rejected() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);
        let badge = signer_badge(&test);

        let component_rule = create_component(
            &test,
            OwnerRule::OwnedBySigner,
            ComponentAccessRules::new().default(zero_of_one(&badge)),
            ResourceAccessRules::new(),
        );
        let resource_updater = create_component(
            &test,
            OwnerRule::OwnedBySigner,
            ComponentAccessRules::new(),
            ResourceAccessRules::new().set_auth_hook_updater(two_of_one(&badge)),
        );
        let nested_owner_rule = create_component(
            &test,
            OwnerRule::ByAccessRule(rule!(resource(TARI_TOKEN)).or(zero_of_one(&badge))),
            ComponentAccessRules::new(),
            ResourceAccessRules::new(),
        );

        let reason = test.execute_expect_failure(component_rule, vec![badge.clone()]);
        assert_reject_reason(reason, invalid_threshold("access_rules", 0, 1));
        let reason = test.execute_expect_failure(resource_updater, vec![badge.clone()]);
        assert_reject_reason(reason, invalid_threshold("resource_access_rules", 2, 1));
        let component_owner_rule = Transaction::builder_localnet(Epoch(1))
            .call_function(
                test.get_template_address("AccessRulesTest"),
                "with_component_owner_rule",
                args![OwnerRule::ByAccessRule(two_of_one(&badge))],
            )
            .build_and_seal(test.secret_key());

        let reason = test.execute_expect_failure(nested_owner_rule, vec![badge.clone()]);
        assert_reject_reason(reason, invalid_threshold("resource_owner_rule", 0, 1));
        let reason = test.execute_expect_failure(component_owner_rule, vec![badge]);
        assert_reject_reason(reason, invalid_threshold("owner_rule", 2, 1));
    }

    #[test]
    fn changing_a_rule_to_an_invalid_threshold_is_rejected() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);
        let badge = signer_badge(&test);
        test.execute_expect_success(
            create_component(
                &test,
                OwnerRule::OwnedBySigner,
                ComponentAccessRules::new().default(AccessRule::AllowAll),
                ResourceAccessRules::new().mintable(AccessRule::DenyAll, UpdateRule::Owner),
            ),
            vec![badge.clone()],
        );
        let component = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let call = |method: &str, args| {
            Transaction::builder_localnet(Epoch(1))
                .call_method(component, method, args)
                .build_and_seal(test.secret_key())
        };
        let set_owner_rule = call("set_component_owner_rule", args![SubstateOwnerRule::ByAccessRule(
            zero_of_one(&badge)
        )]);
        let set_access_rules = call("set_component_access_rules", args![
            ComponentAccessRules::new().method("set_value", two_of_one(&badge))
        ]);
        let update_resource_rule = call("update_tokens_access_rule", args![
            ResourceAuthAction::Mint,
            zero_of_one(&badge)
        ]);

        let reason = test.execute_expect_failure(set_owner_rule, vec![badge.clone()]);
        assert_reject_reason(reason, invalid_threshold("owner_rule", 0, 1));
        let reason = test.execute_expect_failure(set_access_rules, vec![badge.clone()]);
        assert_reject_reason(reason, invalid_threshold("access_rules", 2, 1));
        let reason = test.execute_expect_failure(update_resource_rule, vec![badge]);
        assert_reject_reason(reason, invalid_threshold("new_rule", 0, 1));
    }

    #[test]
    fn a_rule_listing_a_requirement_twice_is_rejected() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);
        let badge = signer_badge(&test);
        let other = NonFungibleAddress::from_public_key(RistrettoPublicKeyBytes::from_bytes(&[1; 32]).unwrap());
        test.execute_expect_success(
            create_component(
                &test,
                OwnerRule::OwnedBySigner,
                ComponentAccessRules::new().default(AccessRule::AllowAll),
                ResourceAccessRules::new(),
            ),
            vec![badge.clone()],
        );
        let component = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let council = rule!(m_of_n(
            2,
            non_fungible(badge.clone()),
            non_fungible(badge.clone()),
            non_fungible(other)
        ));
        let set_owner_rule = Transaction::builder_localnet(Epoch(1))
            .call_method(component, "set_component_owner_rule", args![
                SubstateOwnerRule::ByAccessRule(council)
            ])
            .build_and_seal(test.secret_key());

        let reason = test.execute_expect_failure(set_owner_rule, vec![badge.clone()]);
        assert_reject_reason(reason, RuntimeError::DuplicateMOfNRequirement {
            argument: "owner_rule",
            requirement: RuleRequirement::NonFungibleAddress(badge),
        });
    }
}

mod resource_access_rules {
    use tari_engine::runtime::NativeAction;
    use tari_template_lib::{invoke_args, types::Amount};

    use super::*;

    #[test]
    fn it_denies_actions_on_resource() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        // Create sender and receiver accounts
        let (owner_account, owner_proof, owner_key) = test.create_empty_account();
        let (user_proof, _, user_key) = test.create_owner_proof();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_configured_rules", args![
                    // Owner
                    OwnerRule::OwnedBySigner,
                    // Component
                    ComponentAccessRules::new().default(AccessRule::AllowAll),
                    // Resource
                    ResourceAccessRules::new().withdrawable(AccessRule::DenyAll, OWNER),
                    // Badge recall rule
                    AccessRule::DenyAll,
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        // User cannot get tokens
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        assert_access_denied_for_action(reason, ResourceAuthAction::Withdraw);

        // Owner can get tokens
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        // Owner gives user permission to withdraw tokens
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "update_tokens_access_rule", args![
                    ResourceAuthAction::Withdraw,
                    rule!(non_fungible(user_proof.clone()))
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );

        // User can get tokens, and deposit them in the owners account (deposit is default allow)
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&user_key),
            vec![user_proof],
        );
    }

    #[allow(clippy::too_many_lines)]
    #[test]
    fn it_denies_recall_for_owner() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        // Create sender and receiver accounts
        let (owner_proof, _, owner_key) = test.create_owner_proof();
        let (user_account, user_proof, _) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_configured_rules", args![
                    // Owner - Everyone!
                    OwnerRule::ByAccessRule(AccessRule::AllowAll),
                    // Component
                    ComponentAccessRules::new().default(AccessRule::AllowAll),
                    // Resource
                    ResourceAccessRules::new().withdrawable(rule!(non_fungible(user_proof.clone())), OWNER),
                    // Badge recall rule
                    AccessRule::DenyAll
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        // Give the user a withdraw and deposit badge
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_badge_by_name", args!["withdraw"])
                .put_last_instruction_output_on_workspace("withdraw_perm")
                .call_method(component_address, "take_badge_by_name", args!["deposit"])
                .put_last_instruction_output_on_workspace("deposit_perm")
                .call_method(user_account, "deposit", args![Workspace("withdraw_perm")])
                .call_method(user_account, "deposit", args![Workspace("deposit_perm")])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        // AccessRulesTest field layout (minicbor `#[n(N)]` tags by declaration order):
        // 0: value, 1: tokens, 2: badges, 3: allowed, 4: attack_component.
        let badge_vault: VaultId = test.extract_component_value(component_address, "$.2");
        let badge_resource = *test
            .read_only_state_store()
            .get_vault(&badge_vault)
            .unwrap()
            .resource_address();
        // Account: 0: vaults, 1: approvals.
        let vaults: HashMap<ResourceAddress, VaultId> = test.extract_component_value(user_account, "$.0");
        let user_badge_vault_id = vaults[&badge_resource];

        // Now try recall them. This won't succeed because recall only respects access rules not ownership, so the call
        // is denied for the owner.
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "recall_badge", args![
                    user_badge_vault_id,
                    "withdraw"
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        assert_access_denied_for_action(reason, ResourceAuthAction::Recall);
    }

    #[allow(clippy::too_many_lines)]
    #[test]
    fn it_allows_resource_access_with_badge_then_recall() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        // Create sender and receiver accounts
        let (owner_proof, _, owner_key) = test.create_owner_proof();
        let (user_account, user_proof, user_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "using_badge_rules", args![])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();
        // AccessRulesTest field layout: 0=value, 1=tokens, 2=badges, 3=allowed, 4=attack_component.
        let vault: VaultId = test.extract_component_value(component_address, "$.2");
        // Find the resource address for the badge from the output substates
        let badge_resource = *test
            .read_only_state_store()
            .get_vault(&vault)
            .unwrap()
            .resource_address();
        let vault: VaultId = test.extract_component_value(component_address, "$.1");
        let token_resource = *test
            .read_only_state_store()
            .get_vault(&vault)
            .unwrap()
            .resource_address();

        // User cannot get the tokens
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        assert_access_denied_for_action(reason, ResourceAuthAction::Withdraw);

        // Give the user a withdraw and deposit badge
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_badge_by_name", args!["withdraw"])
                .put_last_instruction_output_on_workspace("withdraw_perm")
                .call_method(component_address, "take_badge_by_name", args!["deposit"])
                .put_last_instruction_output_on_workspace("deposit_perm")
                .call_method(user_account, "deposit", args![Workspace("withdraw_perm")])
                .call_method(user_account, "deposit", args![Workspace("deposit_perm")])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        // User can take tokens
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(user_account, "create_proof_by_non_fungible_ids", args![
                    badge_resource,
                    vec![
                        NonFungibleId::try_from_string("withdraw").unwrap(),
                        NonFungibleId::try_from_string("deposit").unwrap()
                    ]
                ])
                .put_last_instruction_output_on_workspace("proof")
                .call_method(component_address, "get_nft_data_using_proof", args![Workspace("proof")])
                .call_method(component_address, "take_tokens_using_proof", args![
                    Workspace("proof"),
                    10
                ])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit_with_auth", args![
                    Workspace("tokens"),
                    Workspace("proof")
                ])
                .drop_all_proofs_in_workspace()
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        let badge_data = test.return_values()[2].decoded::<Vec<Metadata>>().unwrap();
        assert!(badge_data.iter().all(|b| b.contains_key("colour")));

        let vaults: BTreeMap<ResourceAddress, VaultId> = test.extract_component_value(user_account, "$.0");
        let user_badge_vault_id = vaults[&badge_resource];

        // Recall badge
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "recall_badge", args![
                    user_badge_vault_id,
                    "withdraw"
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        // User can no longer withdraw tokens
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(user_account, "create_proof_for_resource", args![badge_resource])
                .put_last_instruction_output_on_workspace("proof")
                .call_method(user_account, "withdraw", args![token_resource, 10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit", args![Workspace("tokens")])
                .drop_all_proofs_in_workspace()
                .build_and_seal(&user_key),
            vec![user_proof],
        );

        assert_access_denied_for_action(reason, ResourceAuthAction::Withdraw);
    }

    #[test]
    fn it_allows_access_for_proofs_by_amount() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        // Create sender and receiver accounts
        let (owner_proof, _, owner_key) = test.create_owner_proof();
        let (user_account, user_proof, user_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        let result = test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "using_resource_rules", args![])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let access_rules_component = test.return_values()[0].decoded::<ComponentAddress>().unwrap();
        // Find the resource address for the badge from the output substates
        let badge_resource = result
            .finalize
            .result
            .any_accept()
            .unwrap()
            .up_iter()
            .filter_map(|(addr, s)| s.substate_value().as_resource().map(|r| (addr, r)))
            .filter(|(_, r)| r.resource_type().is_non_fungible())
            .map(|(addr, _)| addr.as_resource_address().unwrap())
            .next()
            .unwrap();

        // User cannot get the tokens
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(access_rules_component, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        assert_access_denied_for_action(reason, ResourceAuthAction::Withdraw);

        // Give the user a badge
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(access_rules_component, "mint_new_badge", args![])
                .put_last_instruction_output_on_workspace("permission")
                .call_method(user_account, "deposit", args![Workspace("permission")])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        // User can take tokens
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(user_account, "create_proof_by_amount", args![badge_resource, 1])
                .put_last_instruction_output_on_workspace("proof")
                .call_method(access_rules_component, "take_tokens_using_proof", args![
                    Workspace("proof"),
                    10
                ])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit_with_auth", args![
                    Workspace("tokens"),
                    Workspace("proof")
                ])
                .drop_all_proofs_in_workspace()
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );
    }

    /// A resource rule requires possession. A vault emptied of its badges cannot prove a zero amount or an empty set
    /// of token ids to satisfy it.
    #[test]
    fn an_empty_proof_cannot_satisfy_a_resource_rule() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (owner_proof, _, owner_key) = test.create_owner_proof();
        let (user_account, user_proof, user_key) = test.create_empty_account();
        let (other_account, _, _) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        let result = test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "using_resource_rules", args![])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let access_rules_component = test.return_values()[0].decoded::<ComponentAddress>().unwrap();
        let badge_resource = result
            .finalize
            .result
            .any_accept()
            .unwrap()
            .up_iter()
            .filter_map(|(addr, s)| s.substate_value().as_resource().map(|r| (addr, r)))
            .filter(|(_, r)| r.resource_type().is_non_fungible())
            .map(|(addr, _)| addr.as_resource_address().unwrap())
            .next()
            .unwrap();

        // The user holds a badge, then hands it on, leaving an empty badge vault in their account.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(access_rules_component, "mint_new_badge", args![])
                .put_last_instruction_output_on_workspace("permission")
                .call_method(user_account, "deposit", args![Workspace("permission")])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(user_account, "withdraw", args![badge_resource, 1])
                .put_last_instruction_output_on_workspace("badge")
                .call_method(other_account, "deposit", args![Workspace("badge")])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(user_account, "create_proof_by_amount", args![badge_resource, 0])
                .put_last_instruction_output_on_workspace("proof")
                .call_method(access_rules_component, "take_tokens_using_proof", args![
                    Workspace("proof"),
                    10
                ])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit_with_auth", args![
                    Workspace("tokens"),
                    Workspace("proof")
                ])
                .drop_all_proofs_in_workspace()
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );
        assert_reject_reason(
            reason,
            ResourceError::OperationNotAllowed("A proof must lock a non-zero amount".to_string()),
        );

        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(user_account, "create_proof_by_non_fungible_ids", args![
                    badge_resource,
                    Vec::<NonFungibleId>::new()
                ])
                .put_last_instruction_output_on_workspace("proof")
                .call_method(access_rules_component, "take_tokens_using_proof", args![
                    Workspace("proof"),
                    10
                ])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit_with_auth", args![
                    Workspace("tokens"),
                    Workspace("proof")
                ])
                .drop_all_proofs_in_workspace()
                .build_and_seal(&user_key),
            vec![user_proof],
        );
        assert_reject_reason(
            reason,
            ResourceError::OperationNotAllowed("A proof must lock at least one token id".to_string()),
        );
    }

    /// A resource rule requires possession. A confidential commitment stands in for the resource only because it is
    /// proven to carry value, so a withdraw cannot mint a commitment to zero that passes the rule.
    #[test]
    fn a_zero_value_commitment_cannot_satisfy_a_resource_rule() {
        use tari_ootle_common_types::substate_type::SubstateType;
        use tari_ootle_transaction::TransactionBuilder;
        use tari_template_lib::types::EncryptedData;
        use tari_template_test_tooling::{
            crypto::{RistrettoSecretKey, SecretKey},
            support::{
                confidential::{generate_confidential_output_statement, generate_withdraw_proof},
                value_proof::value_proofs_for_commitment,
            },
            wallet_crypto::{OutputWitness, confidential::create_withdraw_proof},
        };

        let mut test = TemplateTest::new(CRATE_PATH, [
            "tests/templates/access_rules",
            "tests/templates/confidential/faucet",
        ]);

        let (supply, supply_mask, _) = generate_confidential_output_statement(100, None);
        let faucet: ComponentAddress = test.call_function(
            "ConfidentialFaucet",
            "mint",
            args![supply, value_proofs_for_commitment(100u64, &supply_mask)],
            vec![],
        );
        let badge = test
            .get_previous_output_address(SubstateType::Resource)
            .as_resource_address()
            .unwrap();

        let (owner_proof, _, owner_key) = test.create_owner_proof();
        let access_rules_template = test.get_template_address("AccessRulesTest");
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_configured_rules", args![
                    OwnerRule::None,
                    ComponentAccessRules::new().default(AccessRule::AllowAll),
                    ResourceAccessRules::new().withdrawable(rule!(resource(badge)), UpdateRule::Locked),
                    AccessRule::DenyAll,
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );
        let gate = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let take_tokens_with_badge = |builder: TransactionBuilder, account: ComponentAddress| {
            builder
                .call_method(account, "deposit", args![Workspace("badge")])
                .call_method(account, "create_proof_for_resource", args![badge])
                .put_last_instruction_output_on_workspace("proof")
                .call_method(gate, "take_tokens_using_proof", args![Workspace("proof"), 10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(account, "deposit", args![Workspace("tokens")])
                .drop_all_proofs_in_workspace()
        };

        // A commitment holding part of the supply satisfies the rule.
        let (holder_account, holder_proof, holder_key) = test.create_empty_account();
        let withdraw = generate_withdraw_proof(&supply_mask, 1, Some(99), Amount::zero());
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(faucet, "take_free_coins", args![withdraw.proof])
                .put_last_instruction_output_on_workspace("badge")
                .then(|builder| take_tokens_with_badge(builder, holder_account))
                .build_and_seal(&holder_key),
            vec![holder_proof],
        );

        // A commitment to zero, created from no inputs, carries none of the resource.
        let (attacker_account, attacker_proof, attacker_key) = test.create_empty_account();
        let zero_output = OutputWitness {
            amount: 0,
            mask: RistrettoSecretKey::random(&mut rand::rng()),
            sender_public_nonce: Default::default(),
            minimum_value_promise: 0,
            encrypted_data: EncryptedData::try_from(vec![0; EncryptedData::min_size()]).unwrap(),
            resource_view_key: None,
        };
        let zero_proof = create_withdraw_proof(
            &[],
            Amount::zero(),
            Some(&zero_output),
            Amount::zero(),
            None,
            Amount::zero(),
        )
        .unwrap();
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(faucet, "take_free_coins", args![zero_proof])
                .put_last_instruction_output_on_workspace("badge")
                .then(|builder| take_tokens_with_badge(builder, attacker_account))
                .build_and_seal(&attacker_key),
            vec![attacker_proof],
        );
        assert_reject_reason(reason, ResourceError::InvalidConfidentialProof {
            details: "A confidential output must promise a minimum value of at least 1".to_string(),
        });
    }

    /// A resource's withdraw and deposit rules are evaluated in the account's own frame, and a `Proof` argument
    /// is how a badge reaches a frame. The account holds the transaction's signer badge and nothing else of its
    /// own, so a rule naming some other badge is satisfied only by handing that badge in.
    #[test]
    fn the_account_takes_a_badge_restricted_resource_when_handed_the_badge() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (owner_proof, _, owner_key) = test.create_owner_proof();
        let (user_account, user_proof, user_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        let result = test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "using_resource_rules", args![])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let access_rules_component = test.return_values()[0].decoded::<ComponentAddress>().unwrap();
        let resources = result
            .finalize
            .result
            .any_accept()
            .unwrap()
            .up_iter()
            .filter_map(|(addr, s)| s.substate_value().as_resource().map(|r| (addr, r)))
            .map(|(addr, r)| (r.resource_type().is_non_fungible(), addr.as_resource_address().unwrap()))
            .collect::<Vec<_>>();
        let badge_resource = resources.iter().find(|(is_nft, _)| *is_nft).unwrap().1;
        let token_resource = resources.iter().find(|(is_nft, _)| !*is_nft).unwrap().1;

        // Give the user a badge and, with it, some of the restricted tokens. Both the resource's withdraw and
        // deposit rules name the badge.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(access_rules_component, "mint_new_badge", args![])
                .put_last_instruction_output_on_workspace("permission")
                .call_method(user_account, "deposit", args![Workspace("permission")])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(user_account, "create_proof_by_amount", args![badge_resource, 1])
                .put_last_instruction_output_on_workspace("proof")
                .call_method(access_rules_component, "take_tokens_using_proof", args![
                    Workspace("proof"),
                    100
                ])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit_with_auth", args![
                    Workspace("tokens"),
                    Workspace("proof")
                ])
                .drop_all_proofs_in_workspace()
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        // The account frame carries the signer badge, which the resource's withdraw rule does not name.
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(user_account, "withdraw", args![token_resource, 10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        assert_access_denied_for_action(reason, ResourceAuthAction::Withdraw);

        // Handing the badge to the account's frame satisfies it.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(user_account, "create_proof_by_amount", args![badge_resource, 1])
                .put_last_instruction_output_on_workspace("badge")
                .call_method(user_account, "withdraw_with_auth", args![
                    token_resource,
                    10,
                    Workspace("badge")
                ])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit_with_auth", args![
                    Workspace("tokens"),
                    Workspace("badge")
                ])
                .drop_all_proofs_in_workspace()
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        // A proof over the restricted vault is checked against the same rule.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(user_account, "create_proof_by_amount", args![badge_resource, 1])
                .put_last_instruction_output_on_workspace("badge")
                .call_method(user_account, "create_proof_by_amount_with_auth", args![
                    token_resource,
                    10,
                    Workspace("badge")
                ])
                .put_last_instruction_output_on_workspace("token_proof")
                .drop_all_proofs_in_workspace()
                .build_and_seal(&user_key),
            vec![user_proof],
        );
    }

    /// `deposit_with_auth` exists so that someone other than the account's owner can deposit a badge-restricted
    /// resource, handing the account the badge its deposit rule names. Like `deposit`, it must therefore be
    /// callable by anyone: the account's owner rule gates withdrawals, not what arrives.
    #[test]
    fn anyone_may_deposit_with_auth() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (owner_proof, _, owner_key) = test.create_owner_proof();
        let (sender_account, sender_proof, sender_key) = test.create_empty_account();
        let (recipient_account, _, _) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        let result = test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "using_resource_rules", args![])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let access_rules_component = test.return_values()[0].decoded::<ComponentAddress>().unwrap();
        let resources = result
            .finalize
            .result
            .any_accept()
            .unwrap()
            .up_iter()
            .filter_map(|(addr, s)| s.substate_value().as_resource().map(|r| (addr, r)))
            .map(|(addr, r)| (r.resource_type().is_non_fungible(), addr.as_resource_address().unwrap()))
            .collect::<Vec<_>>();
        let badge_resource = resources.iter().find(|(is_nft, _)| *is_nft).unwrap().1;
        let token_resource = resources.iter().find(|(is_nft, _)| !*is_nft).unwrap().1;

        // The sender holds the badge that the token's withdraw and deposit rules name, and some tokens.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(access_rules_component, "mint_new_badge", args![])
                .put_last_instruction_output_on_workspace("permission")
                .call_method(sender_account, "deposit", args![Workspace("permission")])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(sender_account, "create_proof_by_amount", args![badge_resource, 1])
                .put_last_instruction_output_on_workspace("proof")
                .call_method(access_rules_component, "take_tokens_using_proof", args![
                    Workspace("proof"),
                    100
                ])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(sender_account, "deposit_with_auth", args![
                    Workspace("tokens"),
                    Workspace("proof")
                ])
                .drop_all_proofs_in_workspace()
                .build_and_seal(&sender_key),
            vec![sender_proof.clone()],
        );

        // The sender is not the recipient's owner. The deposit is authorized by the badge alone.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(sender_account, "create_proof_by_amount", args![badge_resource, 1])
                .put_last_instruction_output_on_workspace("badge")
                .call_method(sender_account, "withdraw_with_auth", args![
                    token_resource,
                    10,
                    Workspace("badge")
                ])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(recipient_account, "deposit_with_auth", args![
                    Workspace("tokens"),
                    Workspace("badge")
                ])
                .drop_all_proofs_in_workspace()
                .build_and_seal(&sender_key),
            vec![sender_proof],
        );

        let balance: Amount = test.call_method(recipient_account, "balance", args![token_resource], vec![]);
        assert_eq!(balance, Amount::new(10));
    }

    #[test]
    fn it_locks_resources_used_in_proofs() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        // Create sender and receiver accounts
        let (owner_account, owner_proof, owner_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        let result = test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_configured_rules", args![
                    // Owner
                    OwnerRule::OwnedBySigner,
                    // Component
                    ComponentAccessRules::new(),
                    // Resource
                    ResourceAccessRules::new(),
                    // Badge recall rule
                    AccessRule::DenyAll
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();
        // Find the resource address for the tokens from the output substates
        let token_resource = result
            .finalize
            .result
            .any_accept()
            .unwrap()
            .up_iter()
            .filter_map(|(addr, s)| s.substate_value().as_resource().map(|r| (addr, r)))
            .filter(|(_, r)| r.resource_type().is_public_fungible())
            .map(|(addr, _)| addr.as_resource_address().unwrap())
            .next()
            .unwrap();

        // Take some tokens, generate a proof from the bucket (locking them up), and then try withdrawing them
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![1000])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .call_method(owner_account, "create_proof_by_amount", args![token_resource, 1000])
                .put_last_instruction_output_on_workspace("proof")
                .call_method(owner_account, "withdraw", args![token_resource, 1000])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        assert_insufficient_funds_for_action(reason);

        // Drop the proof before withdraw/deposit
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![1000])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .call_method(owner_account, "create_proof_by_amount", args![token_resource, 1000])
                .put_last_instruction_output_on_workspace("proof")
                .drop_all_proofs_in_workspace()
                .call_method(owner_account, "withdraw", args![token_resource, 1000])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );
    }

    #[test]
    fn it_permits_cross_template_calls_using_proofs() {
        let mut test = TemplateTest::new(CRATE_PATH, [
            "tests/templates/access_rules",
            "tests/templates/cross_template",
        ]);

        // Create sender and receiver accounts
        let (owner_account, owner_proof, owner_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        let result = test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "using_resource_rules", args![])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();
        // Find the resource address for the tokens from the output substates
        let badge_resource = result
            .finalize
            .result
            .any_accept()
            .unwrap()
            .up_iter()
            .filter_map(|(addr, s)| s.substate_value().as_resource().map(|r| (addr, r)))
            .filter(|(_, r)| r.resource_type().is_non_fungible())
            .map(|(addr, _)| addr.as_resource_address().unwrap())
            .next()
            .unwrap();

        let cross_call_template = test.get_template_address("CrossTemplate");
        // Try to take tokens without proof. Even though I'm the owner of the resource, the scope does not carry over
        // when cross-template calls are made.
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_function(cross_call_template, "call_component_with_args", args![
                    component_address,
                    "take_tokens",
                    invoke_args![10],
                ])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .drop_all_proofs_in_workspace()
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        assert_access_denied_for_action(reason, ResourceAuthAction::Withdraw);

        // Do a cross template call using a proof
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "mint_new_badge", args![])
                .put_last_instruction_output_on_workspace("badge")
                .call_method(owner_account, "deposit", args![Workspace("badge")])
                .call_method(owner_account, "create_proof_for_resource", args![badge_resource])
                .put_last_instruction_output_on_workspace("proof")
                .call_function(cross_call_template, "call_component_with_args_using_proof", args![
                    component_address,
                    "take_tokens_using_proof",
                    Workspace("proof"),
                    10,
                ])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .drop_all_proofs_in_workspace()
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );
    }

    /// A proof the submitter left on the workspace authorizes the call boundary and nothing past it: a frame acts
    /// with the badges it is stamped with and the proofs it is handed as arguments. A method that takes no `Proof`
    /// therefore cannot reach a resource action the badge guards, however the transaction was assembled.
    #[test]
    fn a_workspace_proof_reaches_a_callee_only_as_an_argument() {
        let mut test = TemplateTest::new(CRATE_PATH, [
            "tests/templates/access_rules",
            "tests/templates/cross_template",
        ]);

        // The component and its resources belong to the owner: the user acts on them with a badge alone, so the
        // resource's owner rule cannot stand in for the badge the test is about.
        let (owner_proof, _, owner_key) = test.create_owner_proof();
        let (user_account, user_proof, user_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");
        let cross_call_template = test.get_template_address("CrossTemplate");

        let result = test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "using_resource_rules", args![])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();
        let badge_resource = result
            .finalize
            .result
            .any_accept()
            .unwrap()
            .up_iter()
            .filter_map(|(addr, s)| s.substate_value().as_resource().map(|r| (addr, r)))
            .filter(|(_, r)| r.resource_type().is_non_fungible())
            .map(|(addr, _)| addr.as_resource_address().unwrap())
            .next()
            .unwrap();

        // Give the user the badge that the token resource's withdraw and deposit rules name.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "mint_new_badge", args![])
                .put_last_instruction_output_on_workspace("badge")
                .call_method(user_account, "deposit", args![Workspace("badge")])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );

        // `take_tokens` takes no proof, so the component's frame is left with its own badges alone.
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(user_account, "create_proof_for_resource", args![badge_resource])
                .put_last_instruction_output_on_workspace("proof")
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit_with_auth", args![
                    Workspace("tokens"),
                    Workspace("proof")
                ])
                .drop_all_proofs_in_workspace()
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        assert_access_denied_for_action(reason, ResourceAuthAction::Withdraw);

        // The same badge, handed to the method, satisfies the rule.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(user_account, "create_proof_for_resource", args![badge_resource])
                .put_last_instruction_output_on_workspace("proof")
                .call_method(component_address, "take_tokens_using_proof", args![
                    Workspace("proof"),
                    10
                ])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit_with_auth", args![
                    Workspace("tokens"),
                    Workspace("proof")
                ])
                .drop_all_proofs_in_workspace()
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        // A frame that was handed the proof may forward it onward.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(user_account, "create_proof_for_resource", args![badge_resource])
                .put_last_instruction_output_on_workspace("proof")
                .call_function(cross_call_template, "call_component_with_args_using_proof", args![
                    component_address,
                    "take_tokens_using_proof",
                    Workspace("proof"),
                    10,
                ])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit_with_auth", args![
                    Workspace("tokens"),
                    Workspace("proof")
                ])
                .drop_all_proofs_in_workspace()
                .build_and_seal(&user_key),
            vec![user_proof],
        );
    }

    /// Signer badges are not proofs: they are stamped into every frame a top-level instruction pushes, so a rule
    /// naming the signer is satisfied without any `Proof` argument.
    #[test]
    fn a_signer_gated_account_method_needs_no_proof_argument() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (owner_account, owner_proof, owner_key) = test.create_funded_account();
        let (other_account, other_proof, other_key) = test.create_empty_account();

        // The account's methods are gated on its owner rule, which the signer badge satisfies with no proof
        // anywhere in the transaction.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(owner_account, "withdraw", args![TARI_TOKEN, 100])
                .put_last_instruction_output_on_workspace("withdrawn")
                .call_method(other_account, "deposit", args![Workspace("withdrawn")])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );

        // A different signer carries a different badge.
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(owner_account, "withdraw", args![TARI_TOKEN, 100])
                .put_last_instruction_output_on_workspace("withdrawn")
                .call_method(other_account, "deposit", args![Workspace("withdrawn")])
                .build_and_seal(&other_key),
            vec![other_proof],
        );

        assert_access_denied_for_action(reason, ActionIdent::ComponentCallMethod {
            component_address: owner_account,
            method: "withdraw".to_string(),
        });
    }

    #[allow(clippy::too_many_lines)]
    #[test]
    fn it_creates_a_proof_from_bucket() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        // Create sender and receiver accounts
        let (owner_proof, _, owner_key) = test.create_owner_proof();
        let (user_account, user_proof, user_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        let result = test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "using_badge_rules", args![])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();
        // Find the resource address for the badge from the output substates
        let badge_resource = result
            .finalize
            .result
            .any_accept()
            .unwrap()
            .up_iter()
            .filter_map(|(addr, s)| s.substate_value().as_resource().map(|r| (addr, r)))
            .filter(|(_, r)| r.resource_type().is_non_fungible())
            .map(|(addr, _)| addr.as_resource_address().unwrap())
            .next()
            .unwrap();

        // User cannot get the tokens
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        assert_access_denied_for_action(reason, ResourceAuthAction::Withdraw);

        // Give the user a withdraw and deposit badge
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_badge_by_name", args!["withdraw"])
                .put_last_instruction_output_on_workspace("withdraw_perm")
                .call_method(component_address, "take_badge_by_name", args!["deposit"])
                .put_last_instruction_output_on_workspace("deposit_perm")
                .call_method(user_account, "deposit", args![Workspace("withdraw_perm")])
                .call_method(user_account, "deposit", args![Workspace("deposit_perm")])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        // Side case: we try deposit back the badges before we drop the proof. This is invalid.
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(
                    user_account,
                    "withdraw_many_non_fungibles",
                    args![
                        badge_resource,
                        vec![
                            NonFungibleId::try_from_string("withdraw").unwrap(),
                            NonFungibleId::try_from_string("deposit").unwrap()
                        ]
                    ],
                )
                .put_last_instruction_output_on_workspace("badges")
                // TODO: this perhaps should be a native instruction
                .call_function(
                    access_rules_template,
                    "create_proof_from_bucket",
                    args![Workspace("badges")],
                )
                .put_last_instruction_output_on_workspace("proof")
                .call_method(
                    component_address,
                    "take_tokens_using_proof",
                    args![Workspace("proof"), 10],
                )
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit_with_auth", args![
                    Workspace("tokens"),
                    Workspace("proof")
                ])
                // Deposit before dropping the proof - this step should error
                .call_method(user_account, "deposit", args![Workspace("badges")])
                .drop_all_proofs_in_workspace()
                .build_and_seal(&owner_key),
            vec![user_proof.clone()],
        );

        assert_reject_reason(reason, RuntimeError::InvalidOpLockedBucket {
            op: "deposit",
            // badges is the 1st bucket
            bucket_id: 0.into(),
            locked_amount: Amount::from(2u64),
        });

        // User can take tokens, using a proof obtained from a bucket
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(
                    user_account,
                    "withdraw_many_non_fungibles",
                    args![
                        badge_resource,
                        vec![
                            NonFungibleId::try_from_string("withdraw").unwrap(),
                            NonFungibleId::try_from_string("deposit").unwrap()
                        ]
                    ],
                )
                .put_last_instruction_output_on_workspace("badges")
                // TODO: this perhaps should be a native instruction
                .call_function(
                    access_rules_template,
                    "create_proof_from_bucket",
                    args![Workspace("badges")],
                )
                .put_last_instruction_output_on_workspace("proof")
                .call_method(
                    component_address,
                    "take_tokens_using_proof",
                    args![Workspace("proof"), 10],
                )
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit_with_auth", args![
                    Workspace("tokens"),
                    Workspace("proof")
                ])
                .drop_all_proofs_in_workspace()
                .call_method(user_account, "deposit", args![Workspace("badges")])
                .build_and_seal(&owner_key),
            vec![user_proof],
        );
    }

    #[test]
    fn it_restricts_resource_actions_to_component() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        // Create sender and receiver accounts
        let (owner_account, owner_proof, owner_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        let result = test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(
                    access_rules_template,
                    "resource_actions_restricted_to_component",
                    args![],
                )
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();
        // Find the resource address for the tokens from the output substates
        let token_resource = result
            .finalize
            .result
            .any_accept()
            .unwrap()
            .up_iter()
            .filter_map(|(addr, s)| s.substate_value().as_resource().map(|r| (addr, r)))
            .filter(|(_, r)| r.resource_type().is_public_fungible())
            .map(|(addr, _)| addr.as_resource_address().unwrap())
            .next()
            .unwrap();

        // Minting using a template function will fail
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "mint_resource", args![token_resource])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        assert_access_denied_for_action(reason, ResourceAuthAction::Mint);

        // Minting in a component context will succeed
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "mint_more_tokens", args![1000])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );
    }

    // The auth-hook path transfers the acting component's identity to the hook method's caller. The
    // hook component is created by `AccessRulesTest::with_auth_hook_gated_on_caller`, whose
    // `caller_gated_hook` method is gated on `caller_component(hook_caller)`. A built-in account acting
    // on the resource (via `deposit`) is the caller the hook observes, even though the account's code
    // never invoked the hook.
    //
    // Because any resource may bind the hook, the caller gate is satisfied whenever the gated account
    // acts on *any* such resource. The hook body must therefore check `AuthHookCaller::resource`
    // against the resources it manages; `caller_gated_hook` does, and that check is mandatory, not
    // optional.
    #[test]
    fn it_transfers_caller_identity_through_auth_hook() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (actor_account, actor_proof, actor_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        // `take_tokens` withdraws into the workspace (the hook is skipped for the component's own
        // resource), then `actor_account.deposit` triggers the hook. The deposit only succeeds if the
        // hook observes `actor_account` as its caller.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_auth_hook_gated_on_caller", args![
                    actor_account
                ])
                .put_last_instruction_output_on_workspace("hook")
                .call_method("hook", "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(actor_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&actor_key),
            vec![actor_proof],
        );
    }

    #[test]
    fn it_denies_auth_hook_when_acting_caller_is_not_gated() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (gated_account, gated_proof, gated_key) = test.create_empty_account();
        let (other_account, other_proof, other_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_auth_hook_gated_on_caller", args![
                    gated_account
                ])
                .build_and_seal(&gated_key),
            vec![gated_proof],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let result = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(other_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&other_key),
            vec![other_proof],
        );

        assert_reject_reason(result, "Resource Auth Hook Denied Access");
    }

    #[test]
    fn it_allows_resource_actions_if_auth_hook_passes() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        // Create sender and receiver accounts
        let (owner_account, owner_proof, owner_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_auth_hook", args![true, "valid_auth_hook"])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );
    }

    #[test]
    fn it_denies_resource_actions_if_auth_hook_fails() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (owner_account, owner_proof, owner_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_auth_hook", args![false, "valid_auth_hook"])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let result = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        assert_reject_reason(result, RuntimeError::AccessDeniedAuthHook {
            action_ident: ResourceAuthAction::Deposit.into(),
            details: "Template error: Access denied for action Deposit".to_string(),
        });
    }

    #[test]
    fn it_disallows_hook_that_writes_to_caller_component() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (_owner_account, owner_proof, owner_key) = test.create_empty_account();
        let (user_account, user_proof, user_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_auth_hook", args![
                    true,
                    "malicious_auth_hook_set_state"
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let result = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        assert_reject_reason(result, RuntimeError::AccessDeniedSetComponentState {
            attempted_on: user_account.into(),
            attempted_by: Box::new(component_address.into()),
        });
    }

    #[test]
    fn it_disallows_hook_that_attempts_mutable_call_to_caller() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (_owner_account, owner_proof, owner_key) = test.create_empty_account();
        let (user_account, user_proof, user_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_auth_hook", args![
                    true,
                    "malicious_auth_hook_call_mut"
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let result = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        assert_reject_reason(result, RuntimeError::ForbiddenInAuthHookContext {
            operation: "call_invoke",
        });
    }

    #[test]
    fn it_allows_hook_to_update_its_own_state() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (_owner_account, owner_proof, owner_key) = test.create_empty_account();
        let (user_account, user_proof, user_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_auth_hook", args![
                    true,
                    "counting_auth_hook"
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit", args![Workspace("tokens")])
                .call_method(component_address, "get_value", args![])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        let value = test.return_values()[3].decoded::<u32>().unwrap();
        assert_eq!(value, 1, "the hook fired once for the deposit");
    }

    #[test]
    fn it_disallows_hook_that_writes_outside_its_own_component() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (_owner_account, owner_proof, owner_key) = test.create_empty_account();
        let (user_account, user_proof, user_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_auth_hook", args![
                    true,
                    "hook_creates_vault"
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let result = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        assert_reject_reason(result, "attempted in a resource auth hook");
    }

    #[test]
    fn it_disallows_hook_that_allocates_an_address() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (_owner_account, owner_proof, owner_key) = test.create_empty_account();
        let (user_account, user_proof, user_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_auth_hook", args![
                    true,
                    "hook_allocates_address"
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let result = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        assert_reject_reason(result, RuntimeError::ForbiddenInAuthHookContext {
            operation: "allocate_address_invoke",
        });
    }

    #[test]
    fn it_disallows_hook_that_mints_into_a_bucket() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (_owner_account, owner_proof, owner_key) = test.create_empty_account();
        let (user_account, user_proof, user_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_untracked_badges_auth_hook", args![
                    "hook_mints_untracked_badges"
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let result = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        assert_reject_reason(result, RuntimeError::ForbiddenInAuthHookContext {
            operation: "new_bucket",
        });
    }

    /// Mint takes the resource's write lock. The hook guards that resource and may legitimately read it, so the
    /// lock must not be held across the hook call.
    #[test]
    fn it_allows_a_hook_to_read_the_resource_it_guards() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (_owner_account, owner_proof, owner_key) = test.create_empty_account();
        let (user_account, user_proof, user_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        let result = test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_mintable_auth_hook", args![
                    "hook_reads_own_resource"
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let token_resource = result
            .finalize
            .result
            .any_accept()
            .unwrap()
            .up_iter()
            .filter_map(|(addr, s)| s.substate_value().as_resource().map(|r| (addr, r)))
            .find(|(_, r)| !r.resource_type().is_non_fungible())
            .map(|(addr, _)| addr.as_resource_address().unwrap())
            .unwrap();

        // `mint_resource` is a function rather than a method, so the mint acts on the resource from outside the
        // hook's own component and the hook runs.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "mint_resource", args![token_resource])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(user_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );
    }

    #[test]
    fn it_disallows_hook_that_attempts_mutable_call_to_another_component_in_the_transaction() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules", "tests/templates/state"]);

        let (_owner_account, owner_proof, owner_key) = test.create_empty_account();
        let (user_account, user_proof, user_key) = test.create_empty_account();

        // User has a state component
        let state_template = test.get_template_address("State");
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(state_template, "restricted", args![])
                .build_and_seal(&user_key),
            vec![owner_proof.clone()],
        );

        let state_component = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_auth_hook_attack_component", args![
                    state_component
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let result = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(state_component, "set", args![1])
                // The hook should not be able to set the state component to 123
                .call_method(user_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        // Check that the access hook fails: a hook frame may not call out to any other component, even though the
        // transaction signer has ownership of the object and the previous call to set works.
        assert_reject_reason(&result, RuntimeError::AccessDeniedAuthHook {
            action_ident: ResourceAuthAction::Deposit.into(),
            details: String::new(),
        });
        assert_reject_reason(&result, RuntimeError::ForbiddenInAuthHookContext {
            operation: "call_invoke",
        });
    }

    #[test]
    fn it_fails_if_auth_hook_is_invalid() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let access_rules_template = test.get_template_address("AccessRulesTest");

        [
            "invalid_auth_hook2",
            "invalid_auth_hook3",
            "invalid_auth_hook4",
            "invalid_auth_hook5",
            "hook_doesnt_exist",
        ]
        .iter()
        .for_each(|hook| {
            let reason = test.execute_expect_failure(
                Transaction::builder_localnet(Epoch(1))
                    .call_function(access_rules_template, "with_auth_hook", args![true, hook])
                    .build_and_seal(test.secret_key()),
                vec![test.owner_proof()],
            );

            assert_reject_reason(reason, RuntimeError::InvalidArgument {
                argument: "CreateResourceArg",
                // Partial error text
                reason: "Authorize hook".to_string(),
            });
        })
    }

    #[test]
    fn auth_hook_is_locked_by_default() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (_, owner_proof, owner_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_auth_hook", args![true, "valid_auth_hook"])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "set_auth_hook", args![None::<String>])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );

        assert_reject_reason(reason, RuntimeError::AccessDenied {
            action_ident: ActionIdent::Native(NativeAction::UpdateResourceAuthHook),
        });
    }

    #[test]
    fn a_denying_auth_hook_can_be_removed_by_the_owner() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (owner_account, owner_proof, owner_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        // `allowed = false` makes `valid_auth_hook` panic on every action, which is the failure this action
        // exists to recover from.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_updatable_auth_hook", args![
                    false,
                    "valid_auth_hook",
                    OWNER
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let take_and_deposit = || {
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&owner_key)
        };

        let reason = test.execute_expect_failure(take_and_deposit(), vec![owner_proof.clone()]);
        assert_reject_reason(reason, RuntimeError::AccessDeniedAuthHook {
            action_ident: ResourceAuthAction::Deposit.into(),
            details: "Template error: Access denied for action Deposit".to_string(),
        });

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "set_auth_hook", args![None::<String>])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        test.execute_expect_success(take_and_deposit(), vec![owner_proof]);
    }

    #[test]
    fn a_replacement_auth_hook_is_in_force() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (owner_account, owner_proof, owner_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        // `caller_gated_hook` permits every action, so the resource starts usable. `allowed = false` only
        // takes effect once `valid_auth_hook` is the hook in force, which is what the swap below installs —
        // so the denial afterwards can only come from the replacement, not from the hook having been dropped.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_updatable_auth_hook", args![
                    false,
                    "caller_gated_hook",
                    OWNER
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let take_and_deposit = || {
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&owner_key)
        };

        test.execute_expect_success(take_and_deposit(), vec![owner_proof.clone()]);

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "set_auth_hook", args![Some("valid_auth_hook")])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let reason = test.execute_expect_failure(take_and_deposit(), vec![owner_proof]);
        assert_reject_reason(reason, RuntimeError::AccessDeniedAuthHook {
            action_ident: ResourceAuthAction::Deposit.into(),
            details: "Template error: Access denied for action Deposit".to_string(),
        });
    }

    #[test]
    fn a_replacement_auth_hook_must_have_a_hook_signature() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (_, owner_proof, owner_key) = test.create_empty_account();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_updatable_auth_hook", args![
                    true,
                    "valid_auth_hook",
                    OWNER
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        for hook in [
            "invalid_auth_hook2",
            "invalid_auth_hook3",
            "invalid_auth_hook4",
            "invalid_auth_hook5",
            "hook_doesnt_exist",
        ] {
            let reason = test.execute_expect_failure(
                Transaction::builder_localnet(Epoch(1))
                    .call_method(component_address, "set_auth_hook", args![Some(hook)])
                    .build_and_seal(&owner_key),
                vec![owner_proof.clone()],
            );

            assert_reject_reason(reason, RuntimeError::InvalidArgument {
                argument: "UpdateAuthHookArg",
                // Partial error text
                reason: "Authorize hook".to_string(),
            });
        }
    }

    #[test]
    fn badge_holder_can_update_an_auth_hook_gated_on_their_badge() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (_, owner_proof, owner_key) = test.create_empty_account();
        let (user_proof, _, user_key) = test.create_owner_proof();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        let updater: UpdateRule = rule!(non_fungible(user_proof.clone())).into();
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_updatable_auth_hook", args![
                    true,
                    "valid_auth_hook",
                    updater
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        // The resource owner does not hold the badge, so they cannot touch the hook.
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "set_auth_hook", args![None::<String>])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );
        assert_reject_reason(reason, RuntimeError::AccessDenied {
            action_ident: ActionIdent::Native(NativeAction::UpdateResourceAuthHook),
        });

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "set_auth_hook", args![None::<String>])
                .build_and_seal(&user_key),
            vec![user_proof],
        );
    }

    #[test]
    fn update_metadata_denied_for_non_owner() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (_, owner_proof, owner_key) = test.create_empty_account();
        let (user_proof, _, user_key) = test.create_owner_proof();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_configured_rules", args![
                    OwnerRule::OwnedBySigner,
                    ComponentAccessRules::new().default(AccessRule::AllowAll),
                    // default update_metadata is DenyAll -> only owner may update
                    ResourceAccessRules::new(),
                    AccessRule::DenyAll,
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let mut new_metadata = Metadata::new();
        new_metadata.insert("description", "updated");

        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "set_tokens_metadata", args![new_metadata])
                .build_and_seal(&user_key),
            vec![user_proof],
        );

        assert_access_denied_for_action(reason, ResourceAuthAction::UpdateMetadata);
    }

    #[test]
    fn update_metadata_allowed_by_custom_rule() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (_, owner_proof, owner_key) = test.create_empty_account();
        let (user_proof, _, user_key) = test.create_owner_proof();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_configured_rules", args![
                    OwnerRule::OwnedBySigner,
                    ComponentAccessRules::new().default(AccessRule::AllowAll),
                    ResourceAccessRules::new().update_metadata(rule!(non_fungible(user_proof.clone())), OWNER),
                    AccessRule::DenyAll,
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        let mut new_metadata = Metadata::new();
        new_metadata.insert("description", "updated by user");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "set_tokens_metadata", args![new_metadata])
                .build_and_seal(&user_key),
            vec![user_proof],
        );
    }

    #[test]
    fn owner_can_update_owner_gated_rule() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (owner_proof, _, owner_key) = test.create_owner_proof();
        let (owner_account, _, _) = test.create_empty_account();
        let (user_proof, _, user_key) = test.create_owner_proof();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        // Withdraw starts denied; the updater is `OWNER`, so the owner can change the rule later.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_configured_rules", args![
                    OwnerRule::OwnedBySigner,
                    ComponentAccessRules::new().default(AccessRule::AllowAll),
                    ResourceAccessRules::new().withdrawable(AccessRule::DenyAll, OWNER),
                    AccessRule::DenyAll,
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        // Owner relaxes the withdraw rule.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "update_tokens_access_rule", args![
                    ResourceAuthAction::Withdraw,
                    rule!(non_fungible(user_proof.clone()))
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );

        // User holding the new badge can now withdraw and deposit.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&user_key),
            vec![user_proof],
        );
    }

    /// A caller requirement is an ordinary badge requirement, so a resource rule may be updated to one: the
    /// resource's minter becomes "whoever is executing on behalf of this component".
    #[test]
    fn update_access_rule_accepts_caller_requirement() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (owner_proof, _, owner_key) = test.create_owner_proof();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_configured_rules", args![
                    OwnerRule::OwnedBySigner,
                    ComponentAccessRules::new().default(AccessRule::AllowAll),
                    ResourceAccessRules::new().mintable(AccessRule::DenyAll, OWNER),
                    AccessRule::DenyAll,
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let component_address = test.return_values()[0].decoded::<ComponentAddress>().unwrap();

        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "update_tokens_access_rule", args![
                    ResourceAuthAction::Mint,
                    rule!(caller_component(component_address))
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );
    }

    #[test]
    fn owner_cannot_update_locked_rule() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (owner_proof, _, owner_key) = test.create_owner_proof();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        // Default ResourceAccessRules leaves the mint updater as `Locked`, so even the owner
        // cannot change the mint rule.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_configured_rules", args![
                    OwnerRule::OwnedBySigner,
                    ComponentAccessRules::new().default(AccessRule::AllowAll),
                    ResourceAccessRules::new(),
                    AccessRule::DenyAll,
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let (component_address, _) = test
            .read_only_state_store()
            .get_components_by_template_address(access_rules_template)
            .unwrap()
            .pop()
            .unwrap();

        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "update_tokens_access_rule", args![
                    ResourceAuthAction::Mint,
                    AccessRule::AllowAll
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof],
        );

        assert_reject_reason(reason, RuntimeError::AccessDenied {
            action_ident: ActionIdent::Native(NativeAction::UpdateResourceAccessRule(ResourceAuthAction::Mint)),
        });
    }

    #[test]
    fn badge_holder_can_update_access_rule_gated_rule() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);

        let (owner_proof, _, owner_key) = test.create_owner_proof();
        let (owner_account, _, _) = test.create_empty_account();
        let (user_proof, _, user_key) = test.create_owner_proof();

        let access_rules_template = test.get_template_address("AccessRulesTest");

        // Withdraw rule starts denied; the updater requires the user's badge — not the owner.
        let updater_rule = rule!(non_fungible(user_proof.clone()));
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(access_rules_template, "with_configured_rules", args![
                    OwnerRule::OwnedBySigner,
                    ComponentAccessRules::new().default(AccessRule::AllowAll),
                    ResourceAccessRules::new().withdrawable(AccessRule::DenyAll, updater_rule),
                    AccessRule::DenyAll,
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );

        let (component_address, _) = test
            .read_only_state_store()
            .get_components_by_template_address(access_rules_template)
            .unwrap()
            .pop()
            .unwrap();

        // Owner cannot update — they do not hold the badge.
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "update_tokens_access_rule", args![
                    ResourceAuthAction::Withdraw,
                    AccessRule::AllowAll
                ])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );
        assert_reject_reason(reason, RuntimeError::AccessDenied {
            action_ident: ActionIdent::Native(NativeAction::UpdateResourceAccessRule(ResourceAuthAction::Withdraw)),
        });

        // Badge holder (the "user" identity) can update.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "update_tokens_access_rule", args![
                    ResourceAuthAction::Withdraw,
                    AccessRule::AllowAll
                ])
                .build_and_seal(&user_key),
            vec![user_proof.clone()],
        );

        // And the relaxed rule is in effect.
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component_address, "take_tokens", args![10])
                .put_last_instruction_output_on_workspace("tokens")
                .call_method(owner_account, "deposit", args![Workspace("tokens")])
                .build_and_seal(&user_key),
            vec![user_proof],
        );
    }
}

mod non_fungible_data {
    use tari_crypto::ristretto::RistrettoSecretKey;
    use tari_engine_types::substate::SubstateId;
    use tari_template_lib::types::NonFungibleAddress;

    use super::*;

    /// Issues a non-fungible resource signed by a fresh key, which owns it, and banks the single token in that
    /// key's account. `None` leaves the builder's default access rules in place.
    fn issue_nft(
        test: &mut TemplateTest,
        resource_rules: Option<ResourceAccessRules>,
    ) -> (ResourceAddress, NonFungibleId, NonFungibleAddress, RistrettoSecretKey) {
        let (owner_account, owner_proof, owner_key) = test.create_empty_account();
        let template = test.get_template_address("AccessRulesTest");

        let result = test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(template, "issue_nft", args![resource_rules])
                .put_last_instruction_output_on_workspace("nft")
                .call_method(owner_account, "deposit", args![Workspace("nft")])
                .build_and_seal(&owner_key),
            vec![owner_proof.clone()],
        );
        let resource = result
            .finalize
            .result
            .accept()
            .unwrap()
            .up_iter()
            .find_map(|(id, _)| id.as_resource_address())
            .unwrap();

        (resource, NonFungibleId::from_u32(1), owner_proof, owner_key)
    }

    /// Calls a free function, so the resource's owner rule can be met only by the signer.
    fn set_nft_data_tx(
        test: &TemplateTest,
        resource: ResourceAddress,
        id: &NonFungibleId,
        value: u32,
        signer: &RistrettoSecretKey,
    ) -> Transaction {
        let template = test.get_template_address("AccessRulesTest");
        Transaction::builder_localnet(Epoch(1))
            .call_function(template, "set_nft_mutable_data", args![resource, id.clone(), value])
            .build_and_seal(signer)
    }

    fn read_nft_data(test: &TemplateTest, resource: ResourceAddress, id: NonFungibleId) -> u32 {
        test.read_only_state_store()
            .get_substate(&SubstateId::NonFungible(NonFungibleAddress::new(resource, id)))
            .unwrap()
            .into_substate_value()
            .into_non_fungible()
            .unwrap()
            .contents()
            .unwrap()
            .decode_mutable_data::<u32>()
            .unwrap()
    }

    #[test]
    fn it_denies_data_updates_to_non_owners_by_default() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);
        let (resource, id, _, _) = issue_nft(&mut test, None);
        let (user_proof, _, user_key) = test.create_owner_proof();

        let tx = set_nft_data_tx(&test, resource, &id, 9999, &user_key);
        let reason = test.execute_expect_failure(tx, vec![user_proof]);

        assert_access_denied_for_action(reason, ResourceAuthAction::UpdateNonFungibleData);
        assert_eq!(read_nft_data(&test, resource, id), 0);
    }

    #[test]
    fn it_allows_the_owner_to_update_data_by_default() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);
        let (resource, id, owner_proof, owner_key) = issue_nft(&mut test, None);

        let tx = set_nft_data_tx(&test, resource, &id, 7, &owner_key);
        test.execute_expect_success(tx, vec![owner_proof]);

        assert_eq!(read_nft_data(&test, resource, id), 7);
    }

    #[test]
    fn it_allows_data_updates_granted_by_an_explicit_rule() {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/access_rules"]);
        let (user_proof, _, user_key) = test.create_owner_proof();
        let (resource, id, _, _) = issue_nft(
            &mut test,
            Some(ResourceAccessRules::new().update_non_fungible_data(rule!(non_fungible(user_proof.clone())), OWNER)),
        );

        let tx = set_nft_data_tx(&test, resource, &id, 42, &user_key);
        test.execute_expect_success(tx, vec![user_proof]);

        assert_eq!(read_nft_data(&test, resource, id), 42);
    }
}
