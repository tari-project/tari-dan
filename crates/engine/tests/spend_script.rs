//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Engine tests for TIP-0006 spend-time conditions: the key path (`spend_key`) and the condition tree
//! (`condition_root` + `SpendWitness::ScriptPath` revealing a `SpendCondition` leaf).

use ootle_byte_type::ToByteType;
use tari_crypto::{
    keys::{PublicKey as _, SecretKey as _},
    ristretto::{RistrettoPublicKey, RistrettoSchnorr, RistrettoSecretKey},
};
use tari_engine_types::{
    limits::STEALTH_LIMITS,
    stealth::{MerkleTree, hashlock_digest},
};
use tari_ootle_common_types::{
    RistrettoSchnorrBlake2bVerifier,
    crypto::create_key_pair_from_seed,
    substate_type::SubstateType,
};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::types::{
    AccessRule,
    Amount,
    FunctionName,
    Hash32,
    ResourceAddress,
    TemplateAddress,
    bytes::Bytes,
    constants::STEALTH_TARI_RESOURCE_ADDRESS,
    crypto::{NoSignatureDomain, PublicKey, RistrettoPublicKeyBytes, Signature},
    stealth::{BuiltinPredicate, Covenant, HashAlg, MerkleProof, SpendCondition, SpendWitness, TemplateFunction},
};
use tari_template_test_tooling::{
    TemplateTest,
    support::{
        assert_error::assert_reject_reason,
        spec::{InputAuthSpec, InputSpec, OutputAuthSpec},
        stealth,
        stealth::StealthSecretTransferData,
    },
    wallet_crypto::MaskAndValue,
};

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");
const TEMPLATE_PATHS: &[&str] = &["tests/templates/stealth", "tests/templates/spend_script"];
const FAUCET_TEMPLATE: &str = "StealthFaucet";
const SCRIPT_TEMPLATE: &str = "SpendScripts";

/// An empty stealth-output set with a concrete element type, for spends that produce only revealed funds.
const NO_OUTPUTS: std::iter::Empty<(u64, OutputAuthSpec)> = std::iter::empty();

fn spend_script(template: TemplateAddress, function: &str, args: Vec<Bytes>) -> TemplateFunction {
    TemplateFunction::new(template, FunctionName::try_from(function).unwrap(), args)
}

/// A `TemplateFunction` spend-condition leaf gating on the given predicate.
fn script_condition(template: TemplateAddress, function: &str, args: Vec<Bytes>) -> SpendCondition {
    SpendCondition::template_function(spend_script(template, function, args))
}

/// A key-path output authorisation owned by `pk`.
fn key_path(pk: RistrettoPublicKeyBytes) -> OutputAuthSpec {
    OutputAuthSpec::KeyPath(pk)
}

/// A condition-tree output authorisation over `leaves`.
fn conditions(leaves: Vec<SpendCondition>) -> OutputAuthSpec {
    OutputAuthSpec::Conditions(leaves)
}

/// A `KeyAndScript` output authorisation: a key path (`spend_key`) plus a condition tree over `leaves`. Used to prove a
/// covenant cannot be escaped by re-committing its `condition_root` while bolting on an unconditional key path.
fn key_and_conditions(spend_key: RistrettoPublicKeyBytes, leaves: Vec<SpendCondition>) -> OutputAuthSpec {
    OutputAuthSpec::KeyAndConditions {
        spend_key,
        conditions: leaves,
    }
}

/// An output spec pairing a value with any authorisation (key path or condition tree).
fn out(value: u64, auth: impl Into<OutputAuthSpec>) -> (u64, OutputAuthSpec) {
    (value, auth.into())
}

fn encode_arg<T: tari_bor::Encode<()>>(value: &T) -> Bytes {
    Bytes::from_vec(tari_bor::encode(value).unwrap())
}

/// Builds the (sealed) `StealthFaucet::new` transaction which mints `mint`'s outputs. Used both for the happy path
/// (expect success) and for rejection tests (expect failure).
fn faucet_new_tx(test: &mut TemplateTest, mint: &StealthSecretTransferData) -> Transaction {
    test.enable_auto_add_proofs_from_signers();
    let faucet_template = test.get_template_address(FAUCET_TEMPLATE);
    let initial_supply = mint.statement.inputs_statement.revealed_amount;
    Transaction::builder_localnet(Epoch(1))
        .call_function(faucet_template, "new", args![
            initial_supply,
            mint.statement.clone(),
            None::<RistrettoPublicKeyBytes>
        ])
        .build_and_seal(test.secret_key())
}

/// Mints a single 100-unit stealth UTXO gated by `auth` and returns the resource address plus the mint secrets (the
/// output mask is needed to spend the UTXO; the recorded auth lets the spend reconstruct its witness).
fn mint_utxo(test: &mut TemplateTest, auth: impl Into<OutputAuthSpec>) -> (ResourceAddress, StealthSecretTransferData) {
    let mint = stealth::generate_mint_statement(vec![out(100, auth)], 0u64, None);
    let tx = faucet_new_tx(test, &mint);
    test.execute_expect_success(tx, vec![]);
    let resx = test
        .get_previous_output_address(SubstateType::Resource)
        .as_resource_address()
        .unwrap();
    (resx, mint)
}

/// Mints one 100-unit stealth UTXO per `auth` (all in a single resource) and returns the resource address plus the mint
/// secrets (`output_masks[i]`/`output_auths[i]` spend the UTXO gated by `auths[i]`).
fn mint_utxos(test: &mut TemplateTest, auths: Vec<OutputAuthSpec>) -> (ResourceAddress, StealthSecretTransferData) {
    let outputs = auths.into_iter().map(|a| out(100, a)).collect::<Vec<_>>();
    let mint = stealth::generate_mint_statement(outputs, 0u64, None);
    let tx = faucet_new_tx(test, &mint);
    test.execute_expect_success(tx, vec![]);
    let resx = test
        .get_previous_output_address(SubstateType::Resource)
        .as_resource_address()
        .unwrap();
    (resx, mint)
}

/// Builds a transfer that spends the single minted UTXO into the provided output authorisation.
fn spend_into(mint: &StealthSecretTransferData, output: impl Into<OutputAuthSpec>) -> StealthSecretTransferData {
    stealth::generate_transfer_data([mint.input_spec_for(0, 100)], 0u64, [out(100, output)], 0u64)
}

// -------------------------------- Key path -------------------------------- //

#[test]
fn key_path_spend_authorised_by_signer_badge() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    // Mint a key-path UTXO owned by the test signer's key, then spend it via the key path. The transaction is sealed by
    // that key, so its signer badge is in scope and authorises the spend.
    let pk = test.to_public_key_bytes();
    let (resx, mint) = mint_utxo(&mut test, key_path(pk));

    let transfer = spend_into(&mint, key_path(pk));
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
}

#[test]
fn key_path_spend_rejected_without_signer_badge() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    // The UTXO is owned by a key the test signer does not control, so no badge for it is in scope.
    let (_secret, foreign) = create_key_pair_from_seed(42);
    let (resx, mint) = mint_utxo(&mut test, key_path(foreign.to_byte_type()));

    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, "was not provided or is not in scope");
}

// -------------------------------- Script path: AccessRule leaf -------------------------------- //

#[test]
fn script_path_access_rule_leaf_allows_spend() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    // An AccessRule leaf checked natively against the auth scope. `allow_all` is satisfied by any spender.
    let (resx, mint) = mint_utxo(
        &mut test,
        conditions(vec![SpendCondition::access_rule(AccessRule::AllowAll)]),
    );

    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
}

#[test]
fn script_path_access_rule_leaf_denies_spend() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    // A DenyAll AccessRule leaf can never be satisfied.
    let (resx, mint) = mint_utxo(
        &mut test,
        conditions(vec![SpendCondition::access_rule(AccessRule::DenyAll)]),
    );

    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, "Access Denied");
}

// -------------------------------- Script path: multi-leaf tree -------------------------------- //

#[test]
fn multi_leaf_tree_spends_via_any_committed_leaf() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    // A condition tree committing several alternative leaves. The spender may satisfy ANY one of them.
    let leaves = vec![
        script_condition(script_template, "always_reject", vec![]),
        script_condition(script_template, "timelock", vec![encode_arg(&u64::MAX)]),
        script_condition(script_template, "always_ok", vec![]),
    ];
    let mint = stealth::generate_mint_statement(vec![out(100, conditions(leaves.clone()))], 0u64, None);
    let tx = faucet_new_tx(&mut test, &mint);
    test.execute_expect_success(tx, vec![]);
    let resx = test
        .get_previous_output_address(SubstateType::Resource)
        .as_resource_address()
        .unwrap();

    // Reveal the satisfiable `always_ok` leaf (index 2), with a real inclusion proof against the multi-leaf root.
    let input = InputSpec::with_auth(
        MaskAndValue {
            mask: mint.output_masks[0].clone(),
            value: 100,
        },
        InputAuthSpec::ScriptPath {
            conditions: leaves.clone(),
            leaf: leaves[2].clone(),
            data: Bytes::default(),
        },
    );
    let transfer =
        stealth::generate_transfer_data([input], 0u64, [out(100, key_path(test.to_public_key_bytes()))], 0u64);
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
}

#[test]
fn revealing_a_leaf_not_in_the_tree_is_rejected() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    // Mint committing a single `always_reject` leaf...
    let committed = vec![script_condition(script_template, "always_reject", vec![])];
    let mint = stealth::generate_mint_statement(vec![out(100, conditions(committed.clone()))], 0u64, None);
    let tx = faucet_new_tx(&mut test, &mint);
    test.execute_expect_success(tx, vec![]);
    let resx = test
        .get_previous_output_address(SubstateType::Resource)
        .as_resource_address()
        .unwrap();

    // ...but attempt to spend by revealing an `always_ok` leaf the output never committed. The inclusion proof (built
    // over a tree containing only `always_ok`) recomputes a different root, so the engine rejects the spend.
    let forged = vec![script_condition(script_template, "always_ok", vec![])];
    let input = InputSpec::with_auth(
        MaskAndValue {
            mask: mint.output_masks[0].clone(),
            value: 100,
        },
        InputAuthSpec::ScriptPath {
            conditions: forged.clone(),
            leaf: forged[0].clone(),
            data: Bytes::default(),
        },
    );
    let transfer =
        stealth::generate_transfer_data([input], 0u64, [out(100, key_path(test.to_public_key_bytes()))], 0u64);
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, "not committed in the condition_root");
}

// -------------------------------- Timelock -------------------------------- //

#[test]
fn timelock_allows_spend_at_or_after_unlock_epoch() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let (resx, mint) = mint_utxo(
        &mut test,
        script_condition(script_template, "timelock", vec![encode_arg(&0u64)]),
    );

    // Output is key-path; the timelock is on the input being spent.
    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
}

#[test]
fn timelock_rejects_spend_before_unlock_epoch() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let (resx, mint) = mint_utxo(
        &mut test,
        script_condition(script_template, "timelock", vec![encode_arg(&u64::MAX)]),
    );

    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, "Spend script rejected the spend");
    assert_reject_reason(&reason, "timelock");
}

// -------------------------------- Recursive covenant -------------------------------- //

#[test]
fn covenant_allows_output_that_preserves_condition() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let covenant = script_condition(script_template, "preserve_covenant", vec![]);
    let (resx, mint) = mint_utxo(&mut test, covenant.clone());

    // The output carries the same covenant condition (so the same condition_root) -> the covenant is satisfied.
    let transfer = spend_into(&mint, covenant);
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
}

#[test]
fn covenant_rejects_output_that_changes_condition() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let covenant = script_condition(script_template, "preserve_covenant", vec![]);
    let (resx, mint) = mint_utxo(&mut test, covenant);

    // The output changes the condition to a key path (different condition_root) -> the covenant rejects the spend.
    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, "Spend script rejected the spend");
}

#[test]
fn covenant_rejects_output_with_added_key_path() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let covenant = script_condition(script_template, "preserve_covenant", vec![]);
    let (resx, mint) = mint_utxo(&mut test, covenant.clone());

    // The output re-commits the covenant's condition_root but bolts on an unconditional key path (KeyAndScript). It
    // would be key-spendable next block, escaping the covenant, so preserving the root alone must not satisfy it.
    let transfer = spend_into(&mint, key_and_conditions(test.to_public_key_bytes(), vec![covenant]));
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, "Spend script rejected the spend");
}

#[test]
fn covenant_rejects_spend_with_no_stealth_outputs() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let covenant = script_condition(script_template, "preserve_covenant", vec![]);
    let (resx, mint) = mint_utxo(&mut test, covenant);

    // Reveal the whole 100 units, producing zero stealth outputs. The covenant requires at least one output that
    // preserves the condition, so the spend is rejected.
    let transfer = stealth::generate_transfer_data([mint.input_spec_for(0, 100)], 0u64, NO_OUTPUTS, 100u64);
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, "Spend script rejected the spend");
}

// -------------------------------- Covenant value conservation -------------------------------- //

/// Spends the single minted UTXO (gated by `covenant`) into the given stealth `outputs`. The input carries `covenant`
/// so the transfer builder emits the covenant balance claim; value assigned to an output with a different condition has
/// left the covenant partition.
fn spend_with_covenant(
    mint: &StealthSecretTransferData,
    covenant: &SpendCondition,
    outputs: Vec<(u64, OutputAuthSpec)>,
) -> StealthSecretTransferData {
    stealth::generate_transfer_data(
        [(
            MaskAndValue {
                mask: mint.output_masks[0].clone(),
                value: 100,
            },
            covenant.clone(),
        )],
        0u64,
        outputs,
        0u64,
    )
}

#[test]
fn covenant_balance_allows_full_conservation() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let covenant = script_condition(script_template, "preserve_balance", vec![]);
    let (resx, mint) = mint_utxo(&mut test, covenant.clone());

    // The full 100 units stay in the covenant -> conserved.
    let transfer = spend_with_covenant(&mint, &covenant, vec![out(100, covenant.clone())]);
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
}

#[test]
fn covenant_balance_rejects_value_leaving_partition() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let covenant = script_condition(script_template, "preserve_balance", vec![]);
    let (resx, mint) = mint_utxo(&mut test, covenant.clone());

    // All 100 units go to a key-path output, leaving the covenant entirely -> rejected (allowance is zero).
    let transfer = spend_with_covenant(&mint, &covenant, vec![out(100, key_path(test.to_public_key_bytes()))]);
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, "Spend script rejected the spend");
}

#[test]
fn covenant_balance_allows_withdrawal_within_allowance() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let covenant = script_condition(script_template, "preserve_balance_with_allowance", vec![encode_arg(
        &30u64,
    )]);
    let (resx, mint) = mint_utxo(&mut test, covenant.clone());

    // Withdraw 30 to a key-path output, keep 70 in the covenant -> within the 30 allowance.
    let transfer = spend_with_covenant(&mint, &covenant, vec![
        out(70, covenant.clone()),
        out(30, key_path(test.to_public_key_bytes())),
    ]);
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
}

#[test]
fn covenant_balance_rejects_withdrawal_over_allowance() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let covenant = script_condition(script_template, "preserve_balance_with_allowance", vec![encode_arg(
        &30u64,
    )]);
    let (resx, mint) = mint_utxo(&mut test, covenant.clone());

    // Withdraw 40 to a key-path output, exceeding the 30 allowance -> rejected.
    let transfer = spend_with_covenant(&mint, &covenant, vec![
        out(60, covenant.clone()),
        out(40, key_path(test.to_public_key_bytes())),
    ]);
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, "Spend script rejected the spend");
}

#[test]
fn covenant_balance_verifies_each_partition_independently() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let covenant_a = script_condition(script_template, "preserve_balance", vec![]);
    let covenant_b = script_condition(script_template, "preserve_balance_with_allowance", vec![encode_arg(
        &50u64,
    )]);
    let (resx, mint) = mint_utxos(&mut test, vec![
        conditions(vec![covenant_a.clone()]),
        conditions(vec![covenant_b.clone()]),
    ]);

    // Two covenant partitions in one transfer. A (input 0) is fully conserved; B (input 1) withdraws 50 within its
    // allowance. Each predicate must match its own claim by partition index, not the other's.
    let transfer = stealth::generate_transfer_data(
        [
            (
                MaskAndValue {
                    mask: mint.output_masks[0].clone(),
                    value: 100,
                },
                covenant_a.clone(),
            ),
            (
                MaskAndValue {
                    mask: mint.output_masks[1].clone(),
                    value: 100,
                },
                covenant_b.clone(),
            ),
        ],
        0u64,
        vec![
            out(100, covenant_a),
            out(50, covenant_b),
            out(50, key_path(test.to_public_key_bytes())),
        ],
        0u64,
    );
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
}

#[test]
fn covenant_balance_rejects_understated_withdrawal() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let covenant = script_condition(script_template, "preserve_balance_with_allowance", vec![encode_arg(
        &30u64,
    )]);
    let (resx, mint) = mint_utxo(&mut test, covenant.clone());

    // Truly move 40 out of the covenant, but tamper the claim to understate the outflow as 30 to feign compliance with
    // the allowance. The proof is bound to the true outflow, so verification fails and the spend is rejected.
    let mut transfer = spend_with_covenant(&mint, &covenant, vec![
        out(60, covenant.clone()),
        out(40, key_path(test.to_public_key_bytes())),
    ]);
    assert_eq!(transfer.statement.covenant_claims.len(), 1);
    transfer.statement.covenant_claims[0].revealed_amount = Amount::from_u64(30);

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, "Spend script rejected the spend");
}

/// A confidential "allowance vault": a real-world covenant where each spend may withdraw at most a fixed cap and the
/// remainder is forced (by the balance covenant) back into a UTXO under the same condition, so the cap keeps applying
/// over the vault's lifetime across successive spends.
#[test]
fn covenant_balance_allowance_vault_persists_across_spends() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let vault = script_condition(script_template, "preserve_balance_with_allowance", vec![encode_arg(
        &30u64,
    )]);
    let recipient = key_path(test.to_public_key_bytes());
    let (resx, mint) = mint_utxo(&mut test, vault.clone());

    // Spend 1: withdraw 30 to the recipient; the remaining 70 rolls back into the vault.
    let spend1 = spend_with_covenant(&mint, &vault, vec![out(70, vault.clone()), out(30, recipient.clone())]);
    let vault_70 = spend1.output_masks[0].clone();
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, spend1.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );

    // Spend 2: spend the persisted 70-unit vault output; withdraw another 30, leaving 40 in the vault.
    let spend2 = stealth::generate_transfer_data(
        [(
            MaskAndValue {
                mask: vault_70,
                value: 70,
            },
            vault.clone(),
        )],
        0u64,
        vec![out(40, vault.clone()), out(30, recipient.clone())],
        0u64,
    );
    let vault_40 = spend2.output_masks[0].clone();
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, spend2.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );

    // The cap re-applies to the persisted vault: withdrawing 35 of the remaining 40 exceeds the 30 allowance.
    let over = stealth::generate_transfer_data(
        [(
            MaskAndValue {
                mask: vault_40,
                value: 40,
            },
            vault.clone(),
        )],
        0u64,
        vec![out(5, vault.clone()), out(35, recipient)],
        0u64,
    );
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, over.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, "Spend script rejected the spend");
}

// -------------------------------- Unconditional reject -------------------------------- //

#[test]
fn always_reject_aborts_spend() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let (resx, mint) = mint_utxo(&mut test, script_condition(script_template, "always_reject", vec![]));

    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, "Spend script rejected the spend");
}

// -------------------------------- Read-only sandbox -------------------------------- //

#[test]
fn read_only_sandbox_blocks_state_mutation() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let (resx, mint) = mint_utxo(&mut test, script_condition(script_template, "try_write", vec![]));

    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, "Spend script rejected the spend");
    assert_reject_reason(&reason, "read-only execution context");
}

/// An event is an output of execution that no later code can observe, so a predicate may emit one without
/// compromising the sandbox.
#[test]
fn sandbox_allows_emit_event() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let (resx, mint) = mint_utxo(&mut test, script_condition(script_template, "emit_event", vec![]));

    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert!(
        result
            .finalize
            .events
            .iter()
            .any(|event| event.topic().ends_with(".spend_script_test")),
        "the predicate's event is in the transaction's events"
    );
}

#[test]
fn sandbox_denies_cross_template_call() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    // The bound arg is a template address for the predicate to call into.
    let (resx, mint) = mint_utxo(
        &mut test,
        script_condition(script_template, "try_cross_template_call", vec![encode_arg(
            &script_template,
        )]),
    );

    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, "Spend script rejected the spend");
    assert_reject_reason(&reason, "call_invoke");
}

// -------------------------------- Compute budget -------------------------------- //

#[test]
fn spend_script_exceeding_compute_budget_aborts() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let (resx, mint) = mint_utxo(&mut test, script_condition(script_template, "exhaust_budget", vec![]));

    // The predicate spins forever; the WASM metering budget aborts it and the engine rejects the spend rather than
    // letting an expensive script stall execution.
    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, "Spend script rejected the spend");
}

// -------------------------------- Signature lock -------------------------------- //

// Must match `SpendSigDomain` / `SIG_MESSAGE` in the spend_script template.
const SPEND_SIG_DOMAIN: &[u8] = b"tari.test.spend_script signature domain";
const SPEND_SIG_MESSAGE: &[u8] = b"spend authorisation";

fn sign_spend(secret: &RistrettoSecretKey, message: &[u8]) -> Signature<NoSignatureDomain> {
    let (nonce, nonce_pub) = RistrettoPublicKey::random_keypair(&mut rand::rng());
    let public_key = RistrettoPublicKey::from_secret_key(secret);
    let challenge = RistrettoSchnorrBlake2bVerifier::compute_challenge(
        SPEND_SIG_DOMAIN,
        message,
        &public_key.to_byte_type(),
        &nonce_pub.to_byte_type(),
    );
    let sig = RistrettoSchnorr::sign_raw_uniform(secret, nonce, &challenge).unwrap();
    sig.to_byte_type().into()
}

#[test]
fn signature_lock_allows_valid_signature() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let (secret, public) = create_key_pair_from_seed(7);
    let public = PublicKey::from(public.to_byte_type());
    let signature = sign_spend(&secret, SPEND_SIG_MESSAGE);

    let condition = script_condition(script_template, "require_signature", vec![
        encode_arg(&public),
        encode_arg(&signature),
    ]);
    let (resx, mint) = mint_utxo(&mut test, condition);

    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
}

#[test]
fn signature_lock_rejects_invalid_signature() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let (secret, public) = create_key_pair_from_seed(7);
    let public = PublicKey::from(public.to_byte_type());
    // Sign a different message -> the bound signature is invalid for SPEND_SIG_MESSAGE.
    let signature = sign_spend(&secret, b"some other message");

    let condition = script_condition(script_template, "require_signature", vec![
        encode_arg(&public),
        encode_arg(&signature),
    ]);
    let (resx, mint) = mint_utxo(&mut test, condition);

    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, "Spend script rejected the spend");
    assert_reject_reason(&reason, "invalid spend signature");
}

// -------------------------------- Spend-time (T2) leaf validation -------------------------------- //

// A condition tree commits only an opaque root, so a `TemplateFunction` leaf is hidden at creation and cannot be
// validated then. The function shape and bound-arg encoding are validated at spend time, when the leaf is revealed.

/// Mints a UTXO gated by a single `function`/`args` leaf (always succeeds — the root is opaque), then spends it
/// revealing that leaf and asserts the spend is rejected with a reason containing `expected`.
fn assert_spend_rejected(function: &str, args: Vec<Bytes>, expected: &str) {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let (resx, mint) = mint_utxo(&mut test, script_condition(script_template, function, args));

    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, expected);
}

#[test]
fn spend_rejects_unknown_function() {
    assert_spend_rejected("does_not_exist", vec![], "not found");
}

#[test]
fn spend_rejects_mutable_function() {
    assert_spend_rejected("bad_mutable", vec![], "must not be mutable");
}

#[test]
fn spend_rejects_non_unit_function() {
    assert_spend_rejected("bad_returns_value", vec![], "must return unit");
}

#[test]
fn spend_rejects_missing_context_arg() {
    assert_spend_rejected("bad_no_context", vec![encode_arg(&0u64)], "must take a SpendContext");
}

#[test]
fn spend_rejects_wrong_bound_arg_count() {
    // `timelock` expects exactly one bound argument; provide none.
    assert_spend_rejected("timelock", vec![], "bound argument");
}

#[test]
fn spend_rejects_malformed_bound_arg() {
    // `timelock` expects one bound argument; provide one whose bytes are not well-formed CBOR (trailing data).
    assert_spend_rejected("timelock", vec![Bytes::from_vec(vec![0x00, 0x00])], "well-formed CBOR");
}

// -------------------------------- Creation-time (T1) validation -------------------------------- //

// An unspendable `{no key, no conditions}` output is no longer expressible: `SpendAuthorization` has no such variant,
// so the illegal state is rejected at compile time (and at the CBOR decode boundary) rather than by a runtime check —
// there is nothing left to assert at the engine level.

// -------------------------------- Weight -------------------------------- //

#[test]
fn script_path_witness_increases_transaction_weight() {
    let test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);

    // A spend unlocked by a large-`args` script-path witness vs an equivalent key-path spend. The weight is computed
    // statically (no execution), so the leaf need not be satisfiable for this comparison.
    let large_leaf = script_condition(script_template, "always_ok", vec![Bytes::from_vec(vec![7u8; 2000])]);
    let mask = RistrettoSecretKey::random(&mut rand::rng());
    let script_spend = stealth::generate_transfer_data(
        [(
            MaskAndValue {
                mask: mask.clone(),
                value: 100,
            },
            large_leaf,
        )],
        0u64,
        NO_OUTPUTS,
        100u64,
    );
    let key_spend = stealth::generate_transfer_data([MaskAndValue { mask, value: 100 }], 0u64, NO_OUTPUTS, 100u64);

    let script_tx = Transaction::builder_localnet(Epoch(1))
        .stealth_transfer(STEALTH_TARI_RESOURCE_ADDRESS, script_spend.statement)
        .finish()
        .seal(test.secret_key());
    let key_tx = Transaction::builder_localnet(Epoch(1))
        .stealth_transfer(STEALTH_TARI_RESOURCE_ADDRESS, key_spend.statement)
        .finish()
        .seal(test.secret_key());

    assert!(
        script_tx.calculate_transaction_weight().as_u64() > key_tx.calculate_transaction_weight().as_u64(),
        "A script-path spend should weigh more than an equivalent key-path spend",
    );
}

// -------------------------------- Builtin predicates -------------------------------- //

fn builtin(predicate: BuiltinPredicate) -> SpendCondition {
    SpendCondition::builtin(predicate)
}

fn covenant(covenant: Covenant) -> SpendCondition {
    SpendCondition::covenant(covenant)
}

/// A conjunction (logical AND) leaf over the atoms of the given single-condition leaves.
fn all(conditions: Vec<SpendCondition>) -> SpendCondition {
    SpendCondition::all(conditions.into_iter().flat_map(|c| c.into_conditions().into_vec()))
}

/// Submits a stealth transfer spending the minted UTXO and asserts success.
fn submit_expect_success(test: &mut TemplateTest, resx: ResourceAddress, transfer: StealthSecretTransferData) {
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
}

/// Submits a stealth transfer spending the minted UTXO and asserts it is rejected with a reason containing `expected`.
fn submit_expect_rejected(
    test: &mut TemplateTest,
    resx: ResourceAddress,
    transfer: StealthSecretTransferData,
    expected: &str,
) {
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    assert_reject_reason(&reason, expected);
}

/// Spends the single minted UTXO via a script path revealing `leaf` from a single-leaf tree, supplying the witness
/// `data` blob the leaf interprets, into `output`.
fn spend_leaf_with_data(
    mint: &StealthSecretTransferData,
    leaf: SpendCondition,
    data: Bytes,
    output: impl Into<OutputAuthSpec>,
) -> StealthSecretTransferData {
    let input = InputSpec::with_auth(
        MaskAndValue {
            mask: mint.output_masks[0].clone(),
            value: 100,
        },
        InputAuthSpec::script_path(vec![leaf.clone()], leaf, data),
    );
    stealth::generate_transfer_data([input], 0u64, [out(100, output)], 0u64)
}

// ---- Timelocks ----

#[test]
fn builtin_after_epoch_allows_when_reached() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (resx, mint) = mint_utxo(&mut test, builtin(BuiltinPredicate::AfterEpoch(0)));
    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    submit_expect_success(&mut test, resx, transfer);
}

#[test]
fn builtin_after_epoch_rejects_before_unlock() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (resx, mint) = mint_utxo(&mut test, builtin(BuiltinPredicate::AfterEpoch(u64::MAX)));
    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    submit_expect_rejected(&mut test, resx, transfer, "Spend condition not met");
}

#[test]
fn builtin_before_epoch_allows_before_deadline() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (resx, mint) = mint_utxo(&mut test, builtin(BuiltinPredicate::BeforeEpoch(u64::MAX)));
    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    submit_expect_success(&mut test, resx, transfer);
}

#[test]
fn builtin_before_epoch_rejects_at_or_after_deadline() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (resx, mint) = mint_utxo(&mut test, builtin(BuiltinPredicate::BeforeEpoch(0)));
    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    submit_expect_rejected(&mut test, resx, transfer, "Spend condition not met");
}

// ---- Covenants ----

#[test]
fn builtin_output_preserves_condition_allows_preserving_output() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let covenant = covenant(Covenant::OutputPreservesCondition);
    let (resx, mint) = mint_utxo(&mut test, covenant.clone());
    let transfer = spend_into(&mint, covenant);
    submit_expect_success(&mut test, resx, transfer);
}

#[test]
fn builtin_output_preserves_condition_rejects_changed_output() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let covenant = covenant(Covenant::OutputPreservesCondition);
    let (resx, mint) = mint_utxo(&mut test, covenant);
    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    submit_expect_rejected(&mut test, resx, transfer, "Spend condition not met");
}

#[test]
fn builtin_output_preserves_condition_rejects_added_key_path() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let covenant = covenant(Covenant::OutputPreservesCondition);
    let (resx, mint) = mint_utxo(&mut test, covenant.clone());
    // The output re-commits the covenant's condition_root but adds a key path (KeyAndScript). Comparing only the root
    // would accept it; the key path is an escape, so the covenant must reject it.
    let transfer = spend_into(&mint, key_and_conditions(test.to_public_key_bytes(), vec![covenant]));
    submit_expect_rejected(&mut test, resx, transfer, "Spend condition not met");
}

#[test]
fn builtin_balance_preserved_allows_full_conservation() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let covenant = covenant(Covenant::BalancePreserved(0));
    let (resx, mint) = mint_utxo(&mut test, covenant.clone());
    let transfer = spend_with_covenant(&mint, &covenant, vec![out(100, covenant.clone())]);
    submit_expect_success(&mut test, resx, transfer);
}

#[test]
fn builtin_balance_preserved_rejects_value_leaving_partition() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let covenant = covenant(Covenant::BalancePreserved(0));
    let (resx, mint) = mint_utxo(&mut test, covenant.clone());
    let transfer = spend_with_covenant(&mint, &covenant, vec![out(100, key_path(test.to_public_key_bytes()))]);
    submit_expect_rejected(&mut test, resx, transfer, "Spend condition not met");
}

#[test]
fn builtin_balance_preserved_rejects_added_key_path() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let covenant = covenant(Covenant::BalancePreserved(0));
    let (resx, mint) = mint_utxo(&mut test, covenant.clone());
    // The conserved value flows to a KeyAndScript output re-committing the covenant root plus a key path. That key path
    // would let it be key-spent next block, escaping conservation, so it must not count as in-partition.
    let transfer = spend_with_covenant(&mint, &covenant, vec![out(
        100,
        key_and_conditions(test.to_public_key_bytes(), vec![covenant.clone()]),
    )]);
    submit_expect_rejected(&mut test, resx, transfer, "Spend condition not met");
}

#[test]
fn builtin_balance_with_allowance_allows_within_cap() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let covenant = covenant(Covenant::BalancePreserved(30));
    let (resx, mint) = mint_utxo(&mut test, covenant.clone());
    let transfer = spend_with_covenant(&mint, &covenant, vec![
        out(70, covenant.clone()),
        out(30, key_path(test.to_public_key_bytes())),
    ]);
    submit_expect_success(&mut test, resx, transfer);
}

#[test]
fn builtin_balance_with_allowance_rejects_over_cap() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let covenant = covenant(Covenant::BalancePreserved(30));
    let (resx, mint) = mint_utxo(&mut test, covenant.clone());
    let transfer = spend_with_covenant(&mint, &covenant, vec![
        out(60, covenant.clone()),
        out(40, key_path(test.to_public_key_bytes())),
    ]);
    submit_expect_rejected(&mut test, resx, transfer, "Spend condition not met");
}

#[test]
fn builtin_output_to_allows_required_output() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let target = SpendCondition::access_rule(AccessRule::AllowAll);
    let target_root = MerkleTree::from_conditions([&target]).unwrap().root();
    let covenant = covenant(Covenant::OutputTo {
        condition_root: target_root,
        min_value: 0,
    });
    let (resx, mint) = mint_utxo(&mut test, covenant);
    let transfer = spend_into(&mint, conditions(vec![target]));
    submit_expect_success(&mut test, resx, transfer);
}

#[test]
fn builtin_output_to_rejects_when_target_absent() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let target = SpendCondition::access_rule(AccessRule::AllowAll);
    let target_root = MerkleTree::from_conditions([&target]).unwrap().root();
    let covenant = covenant(Covenant::OutputTo {
        condition_root: target_root,
        min_value: 0,
    });
    let (resx, mint) = mint_utxo(&mut test, covenant);
    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    submit_expect_rejected(&mut test, resx, transfer, "Spend condition not met");
}

#[test]
fn builtin_output_to_rejects_added_key_path() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let target = SpendCondition::access_rule(AccessRule::AllowAll);
    let target_root = MerkleTree::from_conditions([&target]).unwrap().root();
    let covenant = covenant(Covenant::OutputTo {
        condition_root: target_root,
        min_value: 0,
    });
    let (resx, mint) = mint_utxo(&mut test, covenant);
    // The output commits the required target root but also a key path the payer keeps — not a clean payment to the
    // target, since the payer can reclaim it. OutputTo must reject it.
    let transfer = spend_into(&mint, key_and_conditions(test.to_public_key_bytes(), vec![target]));
    submit_expect_rejected(&mut test, resx, transfer, "Spend condition not met");
}

// ---- Hashlock (witness-carrying) ----

#[test]
fn hashlock_allows_correct_preimage() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let preimage = b"open sesame".to_vec();
    let leaf = builtin(BuiltinPredicate::HashLock {
        hash: hashlock_digest(HashAlg::Sha256, &preimage),
        alg: HashAlg::Sha256,
    });
    let (resx, mint) = mint_utxo(&mut test, conditions(vec![leaf.clone()]));
    let transfer = spend_leaf_with_data(
        &mint,
        leaf,
        Bytes::from_vec(preimage),
        key_path(test.to_public_key_bytes()),
    );
    submit_expect_success(&mut test, resx, transfer);
}

#[test]
fn hashlock_rejects_wrong_preimage() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let leaf = builtin(BuiltinPredicate::HashLock {
        hash: hashlock_digest(HashAlg::Sha256, b"open sesame"),
        alg: HashAlg::Sha256,
    });
    let (resx, mint) = mint_utxo(&mut test, conditions(vec![leaf.clone()]));
    let transfer = spend_leaf_with_data(
        &mint,
        leaf,
        Bytes::from_vec(b"wrong".to_vec()),
        key_path(test.to_public_key_bytes()),
    );
    submit_expect_rejected(&mut test, resx, transfer, "Spend condition not met");
}

#[test]
fn hashlock_rejects_empty_data() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let leaf = builtin(BuiltinPredicate::HashLock {
        hash: hashlock_digest(HashAlg::Sha256, b"open sesame"),
        alg: HashAlg::Sha256,
    });
    let (resx, mint) = mint_utxo(&mut test, conditions(vec![leaf.clone()]));
    // No witness data supplied: the digest of an empty preimage cannot match, so the spend is rejected.
    let transfer = spend_leaf_with_data(&mint, leaf, Bytes::default(), key_path(test.to_public_key_bytes()));
    submit_expect_rejected(&mut test, resx, transfer, "Spend condition not met");
}

// ---- HTLC: hashlock AND key, composed via `All` ----

#[test]
fn htlc_claim_requires_both_preimage_and_signer() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let preimage = b"htlc secret".to_vec();
    // Claim path = reveal the preimage AND prove ownership of the claimant's badge (the test signer). The access rule
    // consumes no witness data, so the hashlock remains the sole consumer of the `data` blob.
    let leaf = all(vec![
        builtin(BuiltinPredicate::HashLock {
            hash: hashlock_digest(HashAlg::Sha256, &preimage),
            alg: HashAlg::Sha256,
        }),
        SpendCondition::access_rule(AccessRule::AllowAll),
    ]);
    let (resx, mint) = mint_utxo(&mut test, conditions(vec![leaf.clone()]));
    let transfer = spend_leaf_with_data(
        &mint,
        leaf,
        Bytes::from_vec(preimage),
        key_path(test.to_public_key_bytes()),
    );
    submit_expect_success(&mut test, resx, transfer);
}

// ---- TemplateFunction reading witness data ----

#[test]
fn template_function_reads_matching_witness_data() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let expected = vec![1u8, 2, 3, 4];
    // The predicate commits `expected` as a bound arg and compares it to the uncommitted witness `data`.
    let leaf = script_condition(script_template, "require_witness_data", vec![encode_arg(&expected)]);
    let (resx, mint) = mint_utxo(&mut test, conditions(vec![leaf.clone()]));
    let transfer = spend_leaf_with_data(
        &mint,
        leaf,
        Bytes::from_vec(expected),
        key_path(test.to_public_key_bytes()),
    );
    submit_expect_success(&mut test, resx, transfer);
}

#[test]
fn template_function_rejects_mismatched_witness_data() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    let leaf = script_condition(script_template, "require_witness_data", vec![encode_arg(&vec![
        1u8, 2, 3, 4,
    ])]);
    let (resx, mint) = mint_utxo(&mut test, conditions(vec![leaf.clone()]));
    let transfer = spend_leaf_with_data(
        &mint,
        leaf,
        Bytes::from_vec(vec![9u8, 9, 9]),
        key_path(test.to_public_key_bytes()),
    );
    submit_expect_rejected(&mut test, resx, transfer, "Spend script rejected the spend");
}

// ---- All conjunction ----

#[test]
fn all_conjunction_allows_when_every_condition_holds() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let leaf = all(vec![
        builtin(BuiltinPredicate::AfterEpoch(0)),
        SpendCondition::access_rule(AccessRule::AllowAll),
    ]);
    let (resx, mint) = mint_utxo(&mut test, conditions(vec![leaf]));
    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    submit_expect_success(&mut test, resx, transfer);
}

#[test]
fn all_conjunction_rejects_when_a_builtin_fails() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let leaf = all(vec![
        builtin(BuiltinPredicate::AfterEpoch(u64::MAX)),
        SpendCondition::access_rule(AccessRule::AllowAll),
    ]);
    let (resx, mint) = mint_utxo(&mut test, conditions(vec![leaf]));
    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    submit_expect_rejected(&mut test, resx, transfer, "Spend condition not met");
}

#[test]
fn all_conjunction_rejects_when_access_rule_fails() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let leaf = all(vec![
        builtin(BuiltinPredicate::AfterEpoch(0)),
        SpendCondition::access_rule(AccessRule::DenyAll),
    ]);
    let (resx, mint) = mint_utxo(&mut test, conditions(vec![leaf]));
    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    submit_expect_rejected(&mut test, resx, transfer, "Access Denied");
}

// ---- Conjunction: structural limits (DoS protection) ----

#[test]
fn empty_conjunction_is_rejected() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (resx, mint) = mint_utxo(&mut test, conditions(vec![all(vec![])]));
    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    submit_expect_rejected(&mut test, resx, transfer, "Empty conjunction");
}

#[test]
fn oversized_conjunction_is_rejected() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let conds =
        vec![SpendCondition::access_rule(AccessRule::AllowAll); STEALTH_LIMITS.max_conditions_per_conjunction + 1];
    let (resx, mint) = mint_utxo(&mut test, conditions(vec![all(conds)]));
    let transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    submit_expect_rejected(&mut test, resx, transfer, "exceeding the limit");
}

#[test]
fn oversized_witness_data_is_rejected() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let leaf = builtin(BuiltinPredicate::HashLock {
        hash: hashlock_digest(HashAlg::Sha256, b"x"),
        alg: HashAlg::Sha256,
    });
    let (resx, mint) = mint_utxo(&mut test, conditions(vec![leaf.clone()]));
    // Build a valid script-path spend, then swap in an over-limit `data` blob (the wallet builder would reject it, so
    // it is injected directly to exercise the engine's bound).
    let mut transfer = spend_leaf_with_data(
        &mint,
        leaf,
        Bytes::from_vec(b"x".to_vec()),
        key_path(test.to_public_key_bytes()),
    );
    if let SpendWitness::ScriptPath { leaf, proof, .. } = transfer.statement.inputs_statement.inputs[0].witness.clone()
    {
        let oversized = Bytes::from_vec(vec![0u8; STEALTH_LIMITS.max_witness_data_len + 1]);
        transfer.statement.inputs_statement.inputs[0].witness =
            SpendWitness::script_path_with_data(leaf, proof, oversized);
    }
    submit_expect_rejected(&mut test, resx, transfer, "exceeding the limit");
}

#[test]
fn oversized_inclusion_proof_is_rejected() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let leaf = builtin(BuiltinPredicate::AfterEpoch(0));
    let (resx, mint) = mint_utxo(&mut test, conditions(vec![leaf]));
    // Build a valid single-leaf script-path spend, then swap in an over-limit inclusion proof. The length check runs
    // before verify_inclusion, so the spend is rejected for the oversized proof rather than for a non-member leaf.
    let mut transfer = spend_into(&mint, key_path(test.to_public_key_bytes()));
    if let SpendWitness::ScriptPath { leaf, data, .. } = transfer.statement.inputs_statement.inputs[0].witness.clone() {
        let oversized = MerkleProof::new(vec![
            Hash32::from_array([0u8; 32]);
            STEALTH_LIMITS.max_inclusion_proof_len + 1
        ]);
        transfer.statement.inputs_statement.inputs[0].witness =
            SpendWitness::script_path_with_data(leaf, oversized, data);
    }
    submit_expect_rejected(&mut test, resx, transfer, "exceeding the limit");
}

// ---- Sole-consumer rule: a data-consuming builtin may not share its leaf with another consumer ----

#[test]
fn data_consuming_builtin_with_template_function_is_rejected() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let script_template = test.get_template_address(SCRIPT_TEMPLATE);
    // A hashlock (data consumer) AND-ed with a TemplateFunction (which may also read `data`) is ambiguous: the blob
    // cannot be both the hashlock's whole preimage and the template's input, so the leaf is rejected at spend time.
    let leaf = all(vec![
        builtin(BuiltinPredicate::HashLock {
            hash: hashlock_digest(HashAlg::Sha256, b"secret"),
            alg: HashAlg::Sha256,
        }),
        script_condition(script_template, "always_ok", vec![]),
    ]);
    let (resx, mint) = mint_utxo(&mut test, conditions(vec![leaf.clone()]));
    let transfer = spend_leaf_with_data(
        &mint,
        leaf,
        Bytes::from_vec(b"secret".to_vec()),
        key_path(test.to_public_key_bytes()),
    );
    submit_expect_rejected(&mut test, resx, transfer, "sole consumer of the witness data");
}

#[test]
fn two_data_consuming_builtins_in_one_leaf_are_rejected() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let leaf = all(vec![
        builtin(BuiltinPredicate::HashLock {
            hash: hashlock_digest(HashAlg::Sha256, b"a"),
            alg: HashAlg::Sha256,
        }),
        builtin(BuiltinPredicate::HashLock {
            hash: hashlock_digest(HashAlg::Sha256, b"b"),
            alg: HashAlg::Sha256,
        }),
    ]);
    let (resx, mint) = mint_utxo(&mut test, conditions(vec![leaf.clone()]));
    let transfer = spend_leaf_with_data(
        &mint,
        leaf,
        Bytes::from_vec(b"a".to_vec()),
        key_path(test.to_public_key_bytes()),
    );
    submit_expect_rejected(&mut test, resx, transfer, "at most one may consume the witness data");
}
