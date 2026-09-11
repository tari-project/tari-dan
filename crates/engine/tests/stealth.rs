//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::BTreeMap;

use ootle_byte_type::ToByteType;
use tari_crypto::{
    commitment::HomomorphicCommitmentFactory,
    keys::PublicKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_engine::runtime::{ActionIdent, NativeAction};
use tari_engine_types::{
    UtxoOutput,
    crypto::{ElgamalVerifiableBalance, ValueLookup, get_commitment_factory},
    resource_container::ResourceError,
};
use tari_ootle_common_types::{crypto::create_key_pair_from_seed, substate_type::SubstateType};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::types::{
    AccessRule,
    ComponentAddress,
    ResourceAddress,
    UtxoAddress,
    UtxoId,
    access_rules::ResourceAuthAction,
    crypto::PedersenCommitmentBytes,
    rule,
    stealth::SpendCondition,
};
use tari_template_test_tooling::{
    TemplateTest,
    support::{
        GenerateValueLookup,
        assert_error::{assert_access_denied_for_action, assert_reject_reason},
        spec::OutputAuthSpec,
        stealth,
        stealth::{NO_INPUTS, StealthSecretTransferData},
        value_proof,
    },
    wallet_crypto::{MaskAndValue, viewable_balance_proof::generate_elgamal_value_proof},
};

const TEMPLATE_PATHS: &[&str] = &["tests/templates/stealth"];
const TEMPLATE_NAME: &str = "StealthFaucet";
const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

fn setup(
    test: &mut TemplateTest,
    transfer_data: &StealthSecretTransferData,
    view_key: Option<&RistrettoPublicKey>,
) -> (ComponentAddress, ResourceAddress) {
    test.enable_auto_add_proofs_from_signers();
    let template_addr = test.get_template_address(TEMPLATE_NAME);
    let initial_supply = transfer_data.statement.inputs_statement.revealed_amount;

    let transaction = Transaction::builder_localnet(Epoch(1))
        .call_function(template_addr, "new", args![
            initial_supply,
            transfer_data.statement,
            view_key.map(|vk| vk.to_byte_type())
        ])
        .build_and_seal(test.secret_key());

    test.execute_expect_success(transaction, vec![]);

    let faucet = test.get_previous_output_address(SubstateType::Component);
    let resx = test.get_previous_output_address(SubstateType::Resource);

    (
        faucet.as_component_address().unwrap(),
        resx.as_resource_address().unwrap(),
    )
}

#[test]
fn mint_initial_supply() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let outputs = vec![100, 1000, 10000];
    let mint = stealth::generate_mint_statement(outputs, 0u64, None);
    let (_faucet, faucet_resx) = setup(&mut test, &mint, None);

    let resource = test.read_only_state_store().get_resource(&faucet_resx).unwrap();
    let total_supply = resource.total_supply().unwrap();
    assert_eq!(total_supply, 11100);
}

#[test]
fn mint_more_later() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let mint = stealth::generate_mint_statement([1200], 0u64, None);
    let (faucet, faucet_resx) = setup(&mut test, &mint, None);

    test.call_method::<()>(faucet, "mint", args![11100], vec![]);

    let resource = test.read_only_state_store().get_resource(&faucet_resx).unwrap();
    let total_supply = resource.total_supply().unwrap();
    assert_eq!(total_supply, 12300);
}

#[test]
fn basic_transfer() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let outputs = vec![100, 1000, 10000];
    let mint = stealth::generate_mint_statement(outputs, 0u64, None);
    let (_faucet, faucet_resx) = setup(&mut test, &mint, None);

    let transfer = stealth::generate_transfer_data(
        [MaskAndValue {
            mask: mint.output_masks[0].clone(),
            value: 100,
        }],
        0u64,
        Some(100),
        0,
    );
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, transfer.statement)
            .finish()
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[0])
            .seal(test.secret_key()),
        vec![],
    );

    let diff = result.finalize.any_accept().unwrap();
    let utxos = diff
        .up_iter()
        .filter_map(|(_, substate)| substate.substate_value().as_utxo())
        .collect::<Vec<_>>();
    assert_eq!(utxos.len(), 1);
    assert!(utxos[0].output().is_some());
}

/// Two independent transfers, both valid on their own, for the fee-intent cap tests. Each spends a different minted
/// UTXO, so the first completes and only the cap can stop the second.
fn two_transfers(mint: &StealthSecretTransferData) -> (StealthSecretTransferData, StealthSecretTransferData) {
    let first = stealth::generate_transfer_data(
        [MaskAndValue {
            mask: mint.output_masks[0].clone(),
            value: 100,
        }],
        0u64,
        Some(100),
        0,
    );
    let second = stealth::generate_transfer_data(
        [MaskAndValue {
            mask: mint.output_masks[1].clone(),
            value: 1000,
        }],
        0u64,
        Some(1000),
        0,
    );
    (first, second)
}

const FEE_INTENT_CAP_REASON: &str = "Maximum number of stealth transfers in the fee intent exceeded";

/// The fee intent runs on free-compute credit, so it may perform only one stealth transfer.
#[test]
fn fee_intent_rejects_a_second_stealth_transfer() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let mint = stealth::generate_mint_statement(vec![100, 1000], 0u64, None);
    let (_faucet, faucet_resx) = setup(&mut test, &mint, None);
    let (first, second) = two_transfers(&mint);

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .with_fee_instructions_builder(|builder| {
                builder
                    .stealth_transfer(faucet_resx, first.statement)
                    .stealth_transfer(faucet_resx, second.statement)
            })
            .finish()
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[0])
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[1])
            .seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, FEE_INTENT_CAP_REASON);
}

/// The cap counts transfers *performed*, so routing the second through a template does not evade it. Were only
/// `StealthTransfer` instructions counted, the WASM route — which costs an invocation and a host call on top of the
/// same verification — would be the way to exceed the limit.
#[test]
fn fee_intent_counts_a_stealth_transfer_performed_from_wasm() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let mint = stealth::generate_mint_statement(vec![100, 1000], 0u64, None);
    let (_faucet, faucet_resx) = setup(&mut test, &mint, None);
    let template_addr = test.get_template_address(TEMPLATE_NAME);
    let (first, second) = two_transfers(&mint);

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .with_fee_instructions_builder(|builder| {
                builder.stealth_transfer(faucet_resx, first.statement).call_function(
                    template_addr,
                    "static_programmatic_transfer",
                    args![faucet_resx, second.statement],
                )
            })
            .finish()
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[0])
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[1])
            .seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, FEE_INTENT_CAP_REASON);
}

/// The cap is scoped to the fee intent: the main intent may perform further transfers, funded by the fee just paid.
#[test]
fn main_intent_may_transfer_after_the_fee_intent_has() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let mint = stealth::generate_mint_statement(vec![100, 1000], 0u64, None);
    let (_faucet, faucet_resx) = setup(&mut test, &mint, None);
    let (first, second) = two_transfers(&mint);

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .with_fee_instructions_builder(|builder| builder.stealth_transfer(faucet_resx, first.statement))
            .stealth_transfer(faucet_resx, second.statement)
            .finish()
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[0])
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[1])
            .seal(test.secret_key()),
        vec![],
    );
}

#[test]
fn programmatic_transfer() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let outputs = vec![100, 1000, 10000];
    let mint = stealth::generate_mint_statement(outputs, 100u64, None);
    let (faucet, _faucet_resx) = setup(&mut test, &mint, None);

    let vault_id = test
        .get_previous_output_address(SubstateType::Vault)
        .as_vault_id()
        .unwrap();

    let transfer = stealth::generate_transfer_data(
        [MaskAndValue {
            mask: mint.output_masks[0].clone(),
            value: 100,
        }],
        0u64,
        Some(75),
        25,
    );
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "programmatic_transfer", args![transfer.statement])
            .finish()
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[0])
            .seal(test.secret_key()),
        vec![],
    );

    let diff = result.finalize.any_accept().unwrap();
    let utxos = diff
        .up_iter()
        .filter_map(|(_, substate)| substate.substate_value().as_utxo())
        .collect::<Vec<_>>();
    assert_eq!(utxos.len(), 1);
    assert!(utxos[0].output().is_some());
    let vault = test.read_only_state_store().get_vault(&vault_id).unwrap();
    assert_eq!(vault.balance(), 125);
}

#[test]
fn transfer_with_revealed_outputs() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let outputs = [100, 1000, 10000];
    let mint = stealth::generate_mint_statement(outputs, 0u64, None);
    let (_faucet, faucet_resx) = setup(&mut test, &mint, None);
    let (account, _proof, _sk) = test.create_empty_account();

    let transfer = stealth::generate_transfer_data(
        [MaskAndValue {
            mask: mint.output_masks[1].clone(),
            value: 1000,
        }],
        0u64,
        [100, 200],
        700,
    );
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, transfer.statement)
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(account, "deposit", args![Workspace("bucket")])
            .finish()
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[1])
            .seal(test.secret_key()),
        vec![],
    );

    let diff = result.finalize.any_accept().unwrap();
    let utxos = diff
        .up_iter()
        .filter_map(|(_, substate)| substate.substate_value().as_utxo())
        .collect::<Vec<_>>();
    assert_eq!(utxos.len(), 2);
    let store = test.read_only_state_store();
    let vaults = store.get_vaults_for_account(account).unwrap();
    let vault = vaults.get(&faucet_resx).unwrap();
    assert_eq!(vault.balance(), 700);
}

#[test]
fn transfer_revealed_between_accounts() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (alice, _alice_proof, alice_sk) = test.create_empty_account();
    let (bob, _proof, _sk) = test.create_empty_account();

    let outputs = [100, 1000, 10000];
    let mint = stealth::generate_mint_statement(outputs, 0u64, None);
    let (_faucet, faucet_resx) = setup(&mut test, &mint, None);

    let transfer_from_faucet = stealth::generate_transfer_data(
        [
            MaskAndValue {
                mask: mint.output_masks[2].clone(),
                value: 10000,
            },
            MaskAndValue {
                mask: mint.output_masks[1].clone(),
                value: 1000,
            },
        ],
        0u64,
        [999, 9901],
        100,
    );
    let transfer_from_alice_to_bob = stealth::generate_transfer_data(NO_INPUTS, 100u64, [25, 25, 25], 25);
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, transfer_from_faucet.statement)
            .put_last_instruction_output_on_workspace("withdrawn_funds_from_stealth_transfer")
            .call_method(alice, "deposit", args![Workspace(
                "withdrawn_funds_from_stealth_transfer"
            )])
            .call_method(alice, "withdraw", args![faucet_resx, 100])
            .put_last_instruction_output_on_workspace("alice_to_bob")
            .stealth_transfer_with_input_bucket(faucet_resx, transfer_from_alice_to_bob.statement, "alice_to_bob")
            .put_last_instruction_output_on_workspace("transfer_to_bob")
            .call_method(bob, "deposit", args![Workspace("transfer_to_bob")])
            .finish()
            // In tests, we set the spend condition to require the mask as a signer
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[1])
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[2])
            .seal(&alice_sk),
        vec![],
    );

    let diff = result.finalize.any_accept().unwrap();
    let utxos = diff
        .up_iter()
        .filter_map(|(_, substate)| substate.substate_value().as_utxo())
        .collect::<Vec<_>>();
    assert_eq!(utxos.len(), 5);
    let store = test.read_only_state_store();
    let vaults = store.get_vaults_for_account(alice).unwrap();
    let vault = vaults.get(&faucet_resx).unwrap();
    assert_eq!(vault.balance(), 0);
    let vaults = store.get_vaults_for_account(bob).unwrap();
    let vault = vaults.get(&faucet_resx).unwrap();
    assert_eq!(vault.balance(), 25);
}

/// A stealth transfer consumes its revealed-funds bucket whole, so funds a proof has locked in that bucket would
/// be destroyed while the proof still names it. A bucket proof locks the whole bucket, so the shape that reaches
/// the drop is a statement with no revealed input: the unlocked amount matches its zero and the locked funds go
/// unexamined.
#[test]
fn transfer_rejects_a_revealed_funds_bucket_with_locked_funds() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template_addr = test.get_template_address(TEMPLATE_NAME);
    let (alice, _alice_proof, alice_sk) = test.create_empty_account();

    let outputs = [100, 1000, 10000];
    let mint = stealth::generate_mint_statement(outputs, 0u64, None);
    let (_faucet, faucet_resx) = setup(&mut test, &mint, None);

    let transfer_from_faucet = stealth::generate_transfer_data(
        [
            MaskAndValue {
                mask: mint.output_masks[2].clone(),
                value: 10000,
            },
            MaskAndValue {
                mask: mint.output_masks[1].clone(),
                value: 1000,
            },
        ],
        0u64,
        [999, 9901],
        100,
    );
    let onward_transfer = stealth::generate_transfer_data(
        [MaskAndValue {
            mask: mint.output_masks[0].clone(),
            value: 100,
        }],
        0u64,
        [40, 60],
        0u64,
    );

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, transfer_from_faucet.statement)
            .put_last_instruction_output_on_workspace("withdrawn")
            .call_method(alice, "deposit", args![Workspace("withdrawn")])
            .call_method(alice, "withdraw", args![faucet_resx, 100])
            .put_last_instruction_output_on_workspace("funds")
            .call_function(template_addr, "lock_bucket", args![Workspace("funds")])
            .put_last_instruction_output_on_workspace("locked")
            .stealth_transfer_with_input_bucket(faucet_resx, onward_transfer.statement, "locked.0")
            // Dropping the proof clears the dangling-proof check, leaving the guard as the only thing between
            // this transfer and a bucket whose locked funds are gone.
            .drop_all_proofs_in_workspace()
            .finish()
            // In tests, we set the spend condition to require the mask as a signer
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[0])
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[1])
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[2])
            .seal(&alice_sk),
        vec![],
    );

    assert_reject_reason(reason, "Cannot stealth transfer from bucket");
}

#[test]
fn transfer_invalid_balance_in_statement() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let outputs = [100, 1000];
    let mint = stealth::generate_mint_statement(outputs, 0u64, None);
    let (_faucet, faucet_resx) = setup(&mut test, &mint, None);
    let (alice, _proof, _sk) = test.create_empty_account();

    let transfer_from_faucet = stealth::generate_transfer_data(
        [MaskAndValue {
            mask: mint.output_masks[0].clone(),
            value: 100,
        }],
        0u64,
        [99],
        // Try to skim a little (1) off the top
        2,
    );
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, transfer_from_faucet.statement)
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(alice, "deposit", args![Workspace("bucket")])
            .finish()
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[0])
            .seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, ResourceError::InvalidBalanceProof {
        details: "Balance proof signature verification failed".to_string(),
    });
}

#[test]
fn transfer_fails_if_transaction_is_not_signed_by_utxo_owner() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let outputs = [100, 1000];
    let mint = stealth::generate_mint_statement(outputs, 0u64, None);
    let (_faucet, faucet_resx) = setup(&mut test, &mint, None);

    let input = MaskAndValue {
        mask: mint.output_masks[0].clone(),
        value: 100,
    };
    let commitment = input.to_commitment();
    let transfer_from_faucet = stealth::generate_transfer_data([input], 0u64, [100], 0);

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, transfer_from_faucet.statement)
            // Missing signer
            // .add_signer(&test.to_public_key_bytes(), &mint.output_masks[0])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let output_0_pk = RistrettoPublicKey::from_secret_key(&mint.output_masks[0]).to_byte_type();

    assert_reject_reason(reason, ResourceError::RequiredSignatureMissingForStealthUtxo {
        commitment: commitment.to_byte_type(),
        public_key: output_0_pk,
    });
}

#[test]
fn transfer_invalid_range_proof_in_statement() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let outputs = [100, 1000];
    let mint = stealth::generate_mint_statement(outputs, 0u64, None);
    let (_faucet, faucet_resx) = setup(&mut test, &mint, None);
    let (alice, _proof, _sk) = test.create_empty_account();

    let mut transfer_from_faucet = stealth::generate_transfer_data(
        [MaskAndValue {
            mask: mint.output_masks[0].clone(),
            value: 100,
        }],
        0u64,
        [99],
        1,
    );
    let mut rp = transfer_from_faucet
        .statement
        .outputs_statement
        .agg_range_proof
        .clone()
        .into_vec();
    rp[100] ^= 0xFF; // Corrupt the range proof
    transfer_from_faucet.statement.outputs_statement.agg_range_proof = rp.try_into().unwrap();

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, transfer_from_faucet.statement)
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(alice, "deposit", args![Workspace("bucket")])
            .finish()
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[0])
            .seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, "Internal range proof(s) error");
}

#[test]
fn many_outputs_in_one_transfer() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    use std::{iter, time::Instant};

    use tari_engine_types::limits;
    // The whole minted amount is spent into equal outputs, so it must divide exactly by the output count or the
    // balance proof will not sum.
    const VALUE_PER_OUTPUT: u64 = 125;
    let max_outputs = limits::STEALTH_LIMITS.max_outputs;
    let total = VALUE_PER_OUTPUT * u64::try_from(max_outputs).unwrap();
    let mint = stealth::generate_mint_statement([total], 0u64, None);
    let (_faucet, faucet_resx) = setup(&mut test, &mint, None);

    let timer = Instant::now();

    let transfer_from_faucet = stealth::generate_transfer_data(
        [MaskAndValue {
            mask: mint.output_masks[0].clone(),
            value: total,
        }],
        0u64,
        iter::repeat_n(VALUE_PER_OUTPUT, max_outputs),
        0,
    );

    // Release mode: ± 23s on M1 Mac, 3.7s on Ryzen 5950x (single thread, total test time 6.1s) for 500 outputs.
    // TODO: verification time (depending on hardware) of 2-10+ seconds is still a problem, determine
    // what the upper bound for utxos should be. Parts of the verification could be parallelized (helps, assuming
    // some minimum CPU spec for a VN). Note that generation in Debug mode took 16 minutes on Ryzen 5950x !
    eprintln!("Generated transfer in {:.2?}", timer.elapsed());

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, transfer_from_faucet.statement)
            .finish()
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[0])
            .seal(test.secret_key()),
        vec![],
    );

    let diff = result.finalize.any_accept().unwrap();
    let utxos = diff
        .up_iter()
        .filter_map(|(_, substate)| substate.substate_value().as_utxo())
        .collect::<Vec<_>>();
    assert_eq!(utxos.len(), max_outputs);
}

pub fn try_brute_force_stealth_balance<L>(
    utxos: &BTreeMap<PedersenCommitmentBytes, UtxoOutput>,
    secret_view_key: &RistrettoSecretKey,
    value_lookup: &L,
) -> Result<Option<u64>, L::Error>
where
    L: ValueLookup,
{
    let decompressed_viewable_balances = utxos
        .values()
        .filter_map(|utxo| utxo.output.viewable_balance.as_ref().map(|vb| vb.try_into().unwrap()))
        .collect::<Vec<_>>();

    let balances =
        ElgamalVerifiableBalance::decrypt_many(secret_view_key, &decompressed_viewable_balances, value_lookup)?;

    // If any of the commitments cannot be decrypted, then we return None
    Ok(balances.into_iter().sum())
}

#[test]
fn mint_with_view_key() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (view_key_secret, view_key) = RistrettoPublicKey::random_keypair(&mut rand::rng());
    let mint = stealth::generate_mint_statement([1000], 0u64, Some(&view_key));
    let (_faucet, faucet_resx) = setup(&mut test, &mint, Some(&view_key));

    let withdraw_proof = stealth::generate_transfer_data_with_view_key(
        [MaskAndValue {
            mask: mint.output_masks[0].clone(),
            value: 1000,
        }],
        0u64,
        [100, 200, 200, 200, 200, 100],
        0,
        &view_key,
    );
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, withdraw_proof.statement)
            .finish()
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[0])
            .seal(test.secret_key()),
        vec![],
    );

    let diff = result.finalize.result.any_accept().unwrap();
    let utxos = diff
        .up_iter()
        .filter_map(|(addr, substate)| {
            addr.as_utxo_address().map(|addr| {
                (
                    addr.id().into_commitment_bytes(),
                    substate.substate_value().as_utxo().unwrap().clone().output.unwrap(),
                )
            })
        })
        .collect();

    let total_balance =
        try_brute_force_stealth_balance(&utxos, &view_key_secret, &GenerateValueLookup::new(0..=200)).unwrap();
    assert_eq!(total_balance, Some(1000));
}

#[test]
fn freeze_then_attempt_spend() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let outputs = vec![100u64, 1000, 10000];
    let mint = stealth::generate_mint_statement(outputs.clone(), 0u64, None);
    let (faucet, faucet_resx) = setup(&mut test, &mint, None);

    let transfer = stealth::generate_transfer_data(
        [
            MaskAndValue {
                mask: mint.output_masks[0].clone(),
                value: 100,
            },
            MaskAndValue {
                mask: mint.output_masks[1].clone(),
                value: 1000,
            },
        ],
        0u64,
        Some(1100),
        0,
    );
    let owner = test.owner_proof();
    let utxos = mint.output_masks
        .iter()
        .zip(outputs)
        .take(2) // Freeze the first two outputs
        .map(|(mask, amount)| {
            let commitment = get_commitment_factory().commit_value(mask, amount);
            UtxoId::from(commitment.to_byte_type())
        })
        .collect::<Vec<_>>();

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "freeze_utxos", args![utxos])
            .build_and_seal(test.secret_key()),
        vec![owner.clone()],
    );

    // Try and spend a frozen output
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, transfer.statement.clone())
            .finish()
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[0])
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[1])
            .seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, ResourceError::InvalidSpend { details: String::new() });

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "unfreeze_utxos", args![utxos])
            .build_and_seal(test.secret_key()),
        vec![owner],
    );

    // Should be able to spend now
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, transfer.statement)
            .finish()
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[0])
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[1])
            .seal(test.secret_key()),
        vec![],
    );

    let diff = result.finalize.any_accept().unwrap();
    let utxos = diff
        .up_iter()
        .filter_map(|(_, substate)| substate.substate_value().as_utxo())
        .collect::<Vec<_>>();
    assert_eq!(utxos.len(), 1);
    assert!(utxos[0].output().is_some());
}

#[test]
fn burn_then_attempt_spend() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let outputs = vec![100u64, 1000, 10000];
    let mint = stealth::generate_mint_statement(outputs.clone(), 0u64, None);
    let (faucet, faucet_resx) = setup(&mut test, &mint, None);

    let transfer = stealth::generate_transfer_data(
        [
            MaskAndValue {
                mask: mint.output_masks[0].clone(),
                value: outputs[0],
            },
            MaskAndValue {
                mask: mint.output_masks[1].clone(),
                value: outputs[1],
            },
        ],
        0u64,
        Some(outputs[0] + outputs[1]),
        0,
    );
    let owner = test.owner_proof();
    let utxos_and_proofs = mint.output_masks
        .iter()
        .zip(outputs)
        .take(2) // Freeze the first two outputs
        .map(|(mask, amount)| {
            let commitment = get_commitment_factory().commit_value(mask, amount);
            let utxo_id = UtxoId::from(commitment.to_byte_type());
            let proof = value_proof::generate_value_proof_mask_knowledge(amount.into(), mask);
            (utxo_id, proof)
        })
        .collect::<Vec<_>>();

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "burn_utxos", args![utxos_and_proofs.clone()])
            .build_and_seal(test.secret_key()),
        vec![owner.clone()],
    );

    // Try and spend a burnt outputs
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, transfer.statement.clone())
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, ResourceError::InvalidSpend { details: String::new() });
    for (utxo_id, _) in utxos_and_proofs {
        let utxo = test
            .read_only_state_store()
            .get_utxo(UtxoAddress::new(faucet_resx, utxo_id))
            .unwrap();
        assert!(utxo.is_burnt());
    }
}

#[test]
fn burn_with_elgamal_value_proof_adjusts_supply() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (view_key_secret, view_key) = RistrettoPublicKey::random_keypair(&mut rand::rng());
    let mint = stealth::generate_mint_statement([100u64, 1000], 0u64, Some(&view_key));
    let (faucet, faucet_resx) = setup(&mut test, &mint, Some(&view_key));

    let resource = test.read_only_state_store().get_resource(&faucet_resx).unwrap();
    assert_eq!(resource.total_supply().unwrap(), 1100);

    // The view-key holder proves the burnt UTXO's value from its viewable balance
    let commitment_bytes = get_commitment_factory()
        .commit_value(&mint.output_masks[1], 1000)
        .to_byte_type();
    let utxo_id = UtxoId::from(commitment_bytes);
    let utxo = test
        .read_only_state_store()
        .get_utxo(UtxoAddress::new(faucet_resx, utxo_id))
        .unwrap();
    let viewable_balance: ElgamalVerifiableBalance = utxo
        .output()
        .unwrap()
        .output
        .viewable_balance
        .as_ref()
        .unwrap()
        .try_into()
        .unwrap();
    let proof = generate_elgamal_value_proof(&view_key_secret, 1000, &commitment_bytes, &viewable_balance);

    let owner = test.owner_proof();
    test.execute_expect_success(
        test.transaction()
            .call_method(faucet, "burn_utxos", args![vec![(utxo_id, proof)]])
            .build_and_seal(test.secret_key()),
        vec![owner],
    );

    let utxo = test
        .read_only_state_store()
        .get_utxo(UtxoAddress::new(faucet_resx, utxo_id))
        .unwrap();
    assert!(utxo.is_burnt());
    let resource = test.read_only_state_store().get_resource(&faucet_resx).unwrap();
    assert_eq!(resource.total_supply().unwrap(), 100);
}

#[test]
fn burn_rejects_elgamal_value_proof_for_a_false_value() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (view_key_secret, view_key) = RistrettoPublicKey::random_keypair(&mut rand::rng());
    let mint = stealth::generate_mint_statement([100u64, 1000], 0u64, Some(&view_key));
    let (faucet, faucet_resx) = setup(&mut test, &mint, Some(&view_key));

    let commitment_bytes = get_commitment_factory()
        .commit_value(&mint.output_masks[1], 1000)
        .to_byte_type();
    let utxo_id = UtxoId::from(commitment_bytes);
    let utxo = test
        .read_only_state_store()
        .get_utxo(UtxoAddress::new(faucet_resx, utxo_id))
        .unwrap();
    let viewable_balance: ElgamalVerifiableBalance = utxo
        .output()
        .unwrap()
        .output
        .viewable_balance
        .as_ref()
        .unwrap()
        .try_into()
        .unwrap();

    // The view-key holder claims a value other than the one encrypted in the viewable balance
    let proof = generate_elgamal_value_proof(&view_key_secret, 1, &commitment_bytes, &viewable_balance);

    let owner = test.owner_proof();
    let reason = test.execute_expect_failure(
        test.transaction()
            .call_method(faucet, "burn_utxos", args![vec![(utxo_id, proof)]])
            .build_and_seal(test.secret_key()),
        vec![owner],
    );
    assert_reject_reason(reason, ResourceError::InvalidValueProof {
        commitment: commitment_bytes,
        details: "Invalid Elgamal encrypted value proof (s.R != K_r + e.D)".to_string(),
    });

    let utxo = test
        .read_only_state_store()
        .get_utxo(UtxoAddress::new(faucet_resx, utxo_id))
        .unwrap();
    assert!(!utxo.is_burnt());
    let resource = test.read_only_state_store().get_resource(&faucet_resx).unwrap();
    assert_eq!(resource.total_supply().unwrap(), 1100);
}

/// The resource-level `withdrawable` rule gates stealth transfers: spending a stealth UTXO of a
/// resource whose withdraw rule requires the issuer's badge is denied unless the issuer authorises
/// the transaction. The minted UTXO's own spend condition is `AllowAll`, so the resource withdraw
/// rule is the only gate under test — distinguishing it from the per-UTXO `SpendAuthorization` path
/// exercised by the `transfer_restricted_by_access_rules_*` tests.
#[test]
fn transfer_denied_by_resource_withdraw_rule() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    test.enable_auto_add_proofs_from_signers();

    let outputs = vec![(100u64, SpendCondition::access_rule(AccessRule::AllowAll))];
    let mint = stealth::generate_mint_statement(outputs, 0u64, None);

    // Create the resource with a withdraw rule requiring the creating signer (the issuer). The
    // in-`new` mint is itself a stealth transfer, so it only succeeds because this construction is
    // sealed by that same issuer key.
    let template_addr = test.get_template_address(TEMPLATE_NAME);
    let initial_supply = mint.statement.inputs_statement.revealed_amount;
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "new_withdraw_gated_by_signer", args![
                initial_supply,
                mint.statement.clone()
            ])
            .build_and_seal(test.secret_key()),
        vec![],
    );
    let faucet_resx = test
        .get_previous_output_address(SubstateType::Resource)
        .as_resource_address()
        .unwrap();

    // Spend the minted (AllowAll) UTXO back into a single confidential output.
    let transfer = stealth::generate_transfer_data(
        [(
            MaskAndValue {
                mask: mint.output_masks[0].clone(),
                value: 100,
            },
            SpendCondition::access_rule(AccessRule::AllowAll),
        )],
        0u64,
        [100u64],
        0,
    );

    // A non-issuer signer cannot authorise the transfer: the resource withdraw rule denies it.
    let (_attacker, _attacker_proof, attacker_sk) = test.create_empty_account();
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, transfer.statement.clone())
            .finish()
            .seal(&attacker_sk),
        vec![],
    );
    assert_access_denied_for_action(reason, ResourceAuthAction::Withdraw);

    // The issuer (the withdraw authority) can: the same transfer now succeeds.
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, transfer.statement)
            .finish()
            .seal(test.secret_key()),
        vec![],
    );

    let diff = result.finalize.any_accept().unwrap();
    let utxos = diff
        .up_iter()
        .filter_map(|(id, substate)| {
            let addr = id.as_utxo_address()?;
            let output = substate.substate_value().as_utxo().and_then(|u| u.output())?;
            Some((addr, output))
        })
        .collect::<Vec<_>>();
    assert_eq!(utxos.len(), 1);
}

#[test]
fn transfer_restricted_by_access_rules_n_of_m() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (_, pk1) = create_key_pair_from_seed(100);
    let pk1 = pk1.to_byte_type();
    let (sk2, pk2) = create_key_pair_from_seed(101);
    let pk2 = pk2.to_byte_type();
    let (sk3, pk3) = create_key_pair_from_seed(102);
    let pk3 = pk3.to_byte_type();
    let (sk4, pk4) = create_key_pair_from_seed(103);
    let pk4 = pk4.to_byte_type();

    // 3-of-4 multisig rule
    let rule = rule!(m_of_n(
        3,
        public_key(pk1),
        public_key(pk2),
        public_key(pk3),
        public_key(pk4)
    ));
    let access_condition = SpendCondition::access_rule(rule);
    let outputs = vec![(100u64, access_condition.clone())];
    let mint = stealth::generate_mint_statement(outputs.clone(), 0u64, None);
    let (_, faucet_resx) = setup(&mut test, &mint, None);

    // The minted UTXO is gated by a condition tree, so it is spent via the script path revealing the AccessRule leaf.
    let transfer = stealth::generate_transfer_data(
        [(
            MaskAndValue {
                mask: mint.output_masks[0].clone(),
                value: outputs[0].0,
            },
            access_condition,
        )],
        0u64,
        [10u64, 90u64],
        0,
    );
    let test_pk = test.to_public_key_bytes();

    // First try to spend with only 2 of the required 3 signatures
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, transfer.statement.clone())
            .finish()
            .add_signer(&test_pk, &sk2)
            .add_signer(&test_pk, &sk3)
            .seal(test.secret_key()),
        vec![],
    );

    assert_access_denied_for_action(reason, ActionIdent::Native(NativeAction::StealthUtxoSpend));

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, transfer.statement)
            .finish()
            .add_signer(&test_pk, &sk2)
            .add_signer(&test_pk, &sk3)
            .add_signer(&test_pk, &sk4)
            .seal(test.secret_key()),
        vec![],
    );

    let diff = result.finalize.any_accept().unwrap();
    let utxos = diff
        .up_iter()
        .filter_map(|(id, substate)| {
            let addr = id.as_utxo_address()?;
            let output = substate.substate_value().as_utxo().and_then(|u| u.output())?;
            Some((addr, output))
        })
        .collect::<Vec<_>>();
    assert_eq!(utxos.len(), 2);
}

#[test]
fn transfer_restricted_by_access_rules_component_scope() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);

    let outputs = vec![(100u64, OutputAuthSpec::KeyPath(test.to_public_key_bytes()))];
    let mint = stealth::generate_mint_statement(outputs, 0u64, None);
    let (component, faucet_resx) = setup(&mut test, &mint, None);

    let component_scope_rule = rule!(component(component));
    let initial_transfer = stealth::generate_transfer_data(
        [MaskAndValue {
            mask: mint.output_masks[0].clone(),
            value: 100,
        }],
        0u64,
        [
            (10u64, SpendCondition::access_rule(component_scope_rule.clone())),
            (90u64, SpendCondition::access_rule(component_scope_rule.clone())),
        ],
        0,
    );

    // Create the new outputs with the component-bound spend condition
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(faucet_resx, initial_transfer.statement.clone())
            .finish()
            .seal(test.secret_key()),
        vec![],
    );

    // Both minted UTXOs are gated by the component-scope condition tree, so each is spent via the script path revealing
    // its AccessRule leaf.
    let transfer = stealth::generate_transfer_data(
        [
            (
                MaskAndValue {
                    mask: initial_transfer.output_masks[0].clone(),
                    value: 10,
                },
                SpendCondition::access_rule(component_scope_rule.clone()),
            ),
            (
                MaskAndValue {
                    mask: initial_transfer.output_masks[1].clone(),
                    value: 90,
                },
                SpendCondition::access_rule(component_scope_rule),
            ),
        ],
        0u64,
        [
            // Anyone with the mask and value (i.e. view key) can spend!
            (99u64, SpendCondition::access_rule(AccessRule::AllowAll)),
        ],
        1, // programmatic transfer in this template requires a revealed output amount
    );

    // First try to spend in a template context
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_function(
                test.get_template_address(TEMPLATE_NAME),
                "static_programmatic_transfer",
                args![faucet_resx, transfer.statement.clone()],
            )
            .finish()
            .seal(test.secret_key()),
        vec![],
    );

    assert_access_denied_for_action(reason, ActionIdent::Native(NativeAction::StealthUtxoSpend));

    // Then, spend in the component context, which succeeds
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(component, "programmatic_transfer", args![transfer.statement])
            .finish()
            .seal(test.secret_key()),
        vec![],
    );

    let diff = result.finalize.any_accept().unwrap();
    let utxos = diff
        .up_iter()
        .filter_map(|(id, substate)| {
            let addr = id.as_utxo_address()?;
            let output = substate.substate_value().as_utxo().and_then(|u| u.output())?;
            Some((addr, output))
        })
        .collect::<Vec<_>>();
    assert_eq!(utxos.len(), 1);
}

#[test]
fn duplicate_inputs_in_one_statement_are_rejected() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let mint = stealth::generate_mint_statement(vec![100u64], 0u64, None);
    let (_faucet, faucet_resx) = setup(&mut test, &mint, None);

    let input = MaskAndValue {
        mask: mint.output_masks[0].clone(),
        value: 100,
    };
    // The excess folds the inputs positionally, so listing the one 100 UTXO twice balances a statement that pays
    // out 200.
    let transfer = stealth::generate_transfer_data([input], 0u64, Some(200), 0);
    let statement = stealth::spend_first_input_twice(&transfer, &mint.output_masks[0]);

    let reason = test.execute_expect_failure(
        test.transaction()
            .stealth_transfer(faucet_resx, statement)
            .finish()
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[0])
            .seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, "Duplicate input commitment");
}

#[test]
fn two_statements_may_not_spend_the_same_utxo() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let mint = stealth::generate_mint_statement(vec![100u64], 0u64, None);
    let (_faucet, faucet_resx) = setup(&mut test, &mint, None);

    let input = MaskAndValue {
        mask: mint.output_masks[0].clone(),
        value: 100,
    };
    let first = stealth::generate_transfer_data([input.clone()], 0u64, Some(100), 0);
    let second = stealth::generate_transfer_data([input], 0u64, Some(100), 0);

    let reason = test.execute_expect_failure(
        test.transaction()
            .stealth_transfer(faucet_resx, first.statement)
            .stealth_transfer(faucet_resx, second.statement)
            .finish()
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[0])
            .seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, "was already spent earlier in this transaction");
}

#[test]
fn a_utxo_spent_in_this_transaction_cannot_also_be_burnt() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let mint = stealth::generate_mint_statement(vec![100u64], 0u64, None);
    let (faucet, faucet_resx) = setup(&mut test, &mint, None);

    let transfer = stealth::generate_transfer_data(
        [MaskAndValue {
            mask: mint.output_masks[0].clone(),
            value: 100,
        }],
        0u64,
        Some(100),
        0,
    );

    let commitment = get_commitment_factory().commit_value(&mint.output_masks[0], 100);
    let utxo_id = UtxoId::from(commitment.to_byte_type());
    let value_proof = value_proof::generate_value_proof_mask_knowledge(100u64.into(), &mint.output_masks[0]);

    // The burn targets the UTXO the transfer has just spent, which is a down and an up@v+1 of one address.
    let reason = test.execute_expect_failure(
        test.transaction()
            .stealth_transfer(faucet_resx, transfer.statement)
            .call_method(faucet, "burn_utxos", args![vec![(utxo_id, value_proof)]])
            .finish()
            .add_signer(&test.to_public_key_bytes(), &mint.output_masks[0])
            .seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, "was already spent earlier in this transaction");
}
