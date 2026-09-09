//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Caller identity is a virtual badge stamped into a frame's authorization scope at push, not a predicate
//! evaluated once at method entry. These tests cover the two properties that follow from that: the badge is
//! checkable wherever any other badge is (so a *resource* rule can gate on the caller), and it names only the
//! immediate caller (so it cannot be forwarded down a call chain).

use tari_engine::runtime::RuntimeError;
use tari_engine_types::substate::SubstateId;
use tari_ootle_transaction::args;
use tari_template_lib::types::{
    ComponentAddress,
    ResourceAddress,
    access_rules::ResourceAuthAction,
    constants::{CALLER_COMPONENT_RESOURCE_ADDRESS, DIRECT_CALLER_TEMPLATE_RESOURCE_ADDRESS},
};
use tari_template_test_tooling::{
    TemplateTest,
    support::assert_error::{assert_access_denied_for_action, assert_reject_reason},
};

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

const TEMPLATES: [&str; 2] = [
    "tests/templates/caller_component_caller",
    "tests/templates/caller_badge_resource",
];

/// `withdrawable(rule!(caller_component(a)))` puts the gate on the resource: a holder whose own methods are
/// `allow_all` may still only move the tokens while it is executing on behalf of `a`.
#[test]
fn resource_withdraw_rule_allows_the_gated_caller() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATES);
    let caller_template = test.get_template_address("Caller");
    let holder_template = test.get_template_address("GatedResource");

    test.execute_expect_success(
        test.transaction()
            .call_function(caller_template, "new", args![])
            .put_last_instruction_output_on_workspace("gate")
            .call_function(holder_template, "new", args![Workspace("gate")])
            .put_last_instruction_output_on_workspace("holder")
            .call_method("gate", "withdraw_from", args![Workspace("holder")])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );
}

/// A top-level `CallMethod` carries no caller badge, so the holder's own `allow_all` method is reached but
/// the withdraw inside it is denied by the resource rule.
#[test]
fn resource_withdraw_rule_denies_a_top_level_signer() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATES);
    let caller_template = test.get_template_address("Caller");
    let holder_template = test.get_template_address("GatedResource");

    test.execute_expect_success(
        test.transaction()
            .call_function(caller_template, "new", args![])
            .put_last_instruction_output_on_workspace("gate")
            .call_function(holder_template, "new", args![Workspace("gate")])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );
    let holder = test
        .read_only_state_store()
        .get_first_component_of(holder_template)
        .unwrap()
        .unwrap();

    let reason = test.execute_expect_failure(
        test.transaction()
            .call_method(holder, "withdraw_once", args![])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );
    assert_access_denied_for_action(reason, ResourceAuthAction::Withdraw);
}

/// A component that is not the gated one is denied, even though it can call the holder's method.
#[test]
fn resource_withdraw_rule_denies_an_unrelated_component() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATES);
    let caller_template = test.get_template_address("Caller");
    let holder_template = test.get_template_address("GatedResource");

    let reason = test.execute_expect_failure(
        test.transaction()
            .call_function(caller_template, "new", args![])
            .put_last_instruction_output_on_workspace("gate")
            .call_function(caller_template, "new", args![])
            .put_last_instruction_output_on_workspace("intruder")
            .call_function(holder_template, "new", args![Workspace("gate")])
            .put_last_instruction_output_on_workspace("holder")
            .call_method("intruder", "withdraw_from", args![Workspace("holder")])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );
    assert_access_denied_for_action(reason, ResourceAuthAction::Withdraw);
}

/// The badge is re-derived at every frame push and never inherited: with `gate -> forwarder -> holder`, the
/// holder's frame carries the forwarder's badge, not the gate's. Without this, any static function or method
/// the gated component transitively reaches would act as a proxy for it.
#[test]
fn caller_badge_is_not_inherited_through_an_intermediate_frame() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATES);
    let caller_template = test.get_template_address("Caller");
    let holder_template = test.get_template_address("GatedResource");

    let reason = test.execute_expect_failure(
        test.transaction()
            .call_function(caller_template, "new", args![])
            .put_last_instruction_output_on_workspace("gate")
            .call_function(caller_template, "new", args![])
            .put_last_instruction_output_on_workspace("forwarder")
            .call_function(holder_template, "new", args![Workspace("gate")])
            .put_last_instruction_output_on_workspace("holder")
            .call_method("gate", "withdraw_via", args![
                Workspace("forwarder"),
                Workspace("holder")
            ])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );
    assert_access_denied_for_action(reason, ResourceAuthAction::Withdraw);
}

/// A static function frame has no component identity, so the template badge is the only one stamped. A
/// `CallFunction` of the gated template must therefore satisfy `direct_caller_template` on a resource rule.
#[test]
fn resource_withdraw_rule_allows_a_static_function_of_the_gated_template() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATES);
    let caller_template = test.get_template_address("Caller");
    let holder_template = test.get_template_address("GatedResource");

    test.execute_expect_success(
        test.transaction()
            .call_function(holder_template, "new_template_gated", args![caller_template])
            .put_last_instruction_output_on_workspace("holder")
            .call_function(caller_template, "withdraw_from_static", args![Workspace("holder")])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );
}

/// The template badge names the immediate caller's template, so a caller from any other template is denied
/// even though its own frame is a perfectly ordinary component frame.
#[test]
fn resource_withdraw_rule_denies_a_caller_from_another_template() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATES);
    let caller_template = test.get_template_address("Caller");
    let holder_template = test.get_template_address("GatedResource");

    // Gated on the holder's own template, which the `Caller` component is not.
    let reason = test.execute_expect_failure(
        test.transaction()
            .call_function(caller_template, "new", args![])
            .put_last_instruction_output_on_workspace("caller")
            .call_function(holder_template, "new_template_gated", args![holder_template])
            .put_last_instruction_output_on_workspace("holder")
            .call_method("caller", "withdraw_from", args![Workspace("holder")])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );
    assert_access_denied_for_action(reason, ResourceAuthAction::Withdraw);
}

/// A resource auth hook runs with the acting component's caller badges in scope, and the acting component never
/// chose the hook code: anyone can bind a hook to a token and deposit that token into any account. The hook frame is
/// therefore confined to its own component state, so those badges cannot be spent on a resource gated on the
/// depositor.
#[test]
fn auth_hook_cannot_spend_the_depositors_caller_badge() {
    let mut test = TemplateTest::new(CRATE_PATH, [
        "tests/templates/caller_badge_resource",
        "tests/templates/caller_badge_hook",
    ]);
    let holder_template = test.get_template_address("GatedResource");
    let attacker_template = test.get_template_address("HookAttacker");
    let (victim, _, _) = test.create_empty_account();

    let result = test.execute_expect_success(
        test.transaction()
            .call_function(holder_template, "new_mint_gated", args![victim])
            .put_last_instruction_output_on_workspace("holder")
            .call_method("holder", "resource_address", args![])
            .put_last_instruction_output_on_workspace("gated")
            .call_function(attacker_template, "new", args![Workspace("gated")])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );
    let gated: ResourceAddress = result.finalize.execution_results[2].decode().unwrap();
    let attacker: ComponentAddress = result.finalize.execution_results[4].decode().unwrap();

    let reason = test.execute_expect_failure(
        test.transaction()
            .call_method(attacker, "take_junk", args![])
            .put_last_instruction_output_on_workspace("junk")
            .call_method(victim, "deposit", args![Workspace("junk")])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );
    assert_reject_reason(&reason, RuntimeError::WriteOutsideOwnComponent {
        id: SubstateId::Resource(gated),
    });
}

/// A caller badge is issued into an authorization scope and is not backed by a resource, so there is no path
/// from a badge to a `Proof` that a callee could capture and forward. Both badge resource addresses must stay
/// empty for that to hold: a mint must fail because there is nothing at the address, not for any other reason.
#[test]
fn caller_badge_resources_hold_no_tokens() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/caller_badge_mint_attempt"]);
    let template = test.get_template_address("BadgeMintAttempt");

    for (function, resource) in [
        ("mint_caller_component_badge", CALLER_COMPONENT_RESOURCE_ADDRESS),
        (
            "mint_direct_caller_template_badge",
            DIRECT_CALLER_TEMPLATE_RESOURCE_ADDRESS,
        ),
    ] {
        let reason = test.execute_expect_failure(
            test.transaction()
                .call_function(template, function, args![])
                .build_and_seal(test.secret_key()),
            vec![test.owner_proof()],
        );
        assert_reject_reason(reason, format!("{resource} not found"));
    }
}
