//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! A wallet that verifies its indexer marks a stealth output it holds as spent only once the indexer proves that the
//! output has gone down.

mod support;

use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::{Epoch, StateVersion, SubstateVersion, shard::Shard};
use tari_ootle_light_client::test_fixtures::{NETWORK, Signers, ValidatorSetFixture, substate_down_proof};
use tari_ootle_wallet_sdk::{
    IndexerTrust,
    models::{
        EndOfShard,
        KeyBranch,
        KeyId,
        OutputStatus,
        StartOfShard,
        StealthOutputModel,
        UtxoSpent,
        UtxoUpdatePayload,
        WalletUtxoUpdate,
    },
};
use tari_ootle_wallet_sdk_services::{
    notify::Notify,
    utxo_scanner::{StealthScannerApiError, StealthUtxoScannerWorker, UtxoScanRoundStats},
};
use tari_template_lib::types::{
    Amount,
    EncryptedData,
    constants::STEALTH_TARI_RESOURCE_ADDRESS,
    crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes, UtxoTag},
    stealth::SpendAuthorization,
};

use crate::support::{ProvingNetwork, Test, TestWithNetwork};

const VALUE: u64 = 123;
const VERSION: SubstateVersion = SubstateVersion::ZERO;
const SHARD: Shard = Shard::first();
const SCANNED_TO: StateVersion = StateVersion::new(7);
/// The key strings of [`OutputStatus::Unspent`] and [`OutputStatus::Spent`].
const UNSPENT: &str = "Unspent";
const SPENT: &str = "Spent";

fn held_output(status: OutputStatus) -> StealthOutputModel {
    StealthOutputModel {
        owner_account: Test::test_account_address(),
        resource_address: STEALTH_TARI_RESOURCE_ADDRESS,
        commitment: PedersenCommitmentBytes::from_array([1; PedersenCommitmentBytes::length()]),
        value: VALUE,
        sender_public_nonce: RistrettoPublicKeyBytes::default(),
        view_only_key_id: KeyId::derived(KeyBranch::ViewOnlyKey, 0),
        owner_key_id: Some(KeyId::derived(KeyBranch::Account, 0)),
        encrypted_data: EncryptedData::try_from(vec![0u8; EncryptedData::min_size()]).unwrap(),
        tag_byte: UtxoTag::new(1),
        memo: None,
        auth: SpendAuthorization::Key(RistrettoPublicKeyBytes::default()),
        minimum_value_promise: 0,
        status,
        is_burnt: false,
        is_frozen: false,
        is_on_chain: true,
        is_condition_spendable: true,
        lock_id: None,
    }
}

fn substate_id(output: &StealthOutputModel) -> SubstateId {
    SubstateId::Utxo(output.to_utxo_address())
}

/// A wallet holding `output`, anchored on the validator set of epoch 2 and walked to that of epoch 5.
async fn wallet_holding(
    output: &StealthOutputModel,
    indexer_trust: IndexerTrust,
) -> (
    Vec<ValidatorSetFixture>,
    ProvingNetwork,
    TestWithNetwork<ProvingNetwork>,
) {
    let sets = (1..=5).map(ValidatorSetFixture::new).collect::<Vec<_>>();
    let network = ProvingNetwork::new(&sets, 0);
    let test = TestWithNetwork::with_network_trusting(network.clone(), indexer_trust);
    test.sdk().validator_set_api().sync().await.unwrap();
    network.set_latest(3);
    let trusted = test.sdk().validator_set_api().sync().await.unwrap();
    assert_eq!(trusted.epoch(), Epoch(5));
    test.sdk().stealth_outputs_api().add_output(output).unwrap();
    (sets, network, test)
}

/// The indexer reports `output` spent at `version` to every scan of [`SHARD`] from below [`SCANNED_TO`].
fn report_spent(network: &ProvingNetwork, output: &StealthOutputModel, version: SubstateVersion) {
    let id = output.commitment.into();
    network.answer_utxo_streams(move |cursors| {
        if cursors
            .iter()
            .any(|(shard, cursor)| *shard == SHARD && *cursor >= SCANNED_TO)
        {
            return vec![];
        }
        vec![UtxoUpdatePayload {
            sos: Some(StartOfShard {
                shard: SHARD,
                max_state_version: SCANNED_TO,
                has_more: false,
            }),
            update: Some(WalletUtxoUpdate::Spent(UtxoSpent { id, version })),
            eos: Some(EndOfShard {
                max_state_version: SCANNED_TO,
            }),
        }]
    });
}

async fn try_scan(test: &TestWithNetwork<ProvingNetwork>) -> Result<UtxoScanRoundStats, StealthScannerApiError> {
    let (_join, scanner) = StealthUtxoScannerWorker::new(test.sdk().clone(), Notify::new(100)).spawn();
    scanner
        .scan(Test::test_account_address(), STEALTH_TARI_RESOURCE_ADDRESS)
        .await
}

async fn scan(test: &TestWithNetwork<ProvingNetwork>) -> UtxoScanRoundStats {
    try_scan(test).await.unwrap()
}

fn stored_status(test: &TestWithNetwork<ProvingNetwork>, output: &StealthOutputModel) -> &'static str {
    test.sdk()
        .stealth_outputs_api()
        .utxos_get_many(
            &STEALTH_TARI_RESOURCE_ADDRESS,
            Some(&Test::test_account_address()),
            None,
        )
        .unwrap()
        .into_iter()
        .find(|stored| stored.commitment == output.commitment)
        .expect("the output is stored")
        .status
        .as_key_str()
}

fn unspent_balance(test: &TestWithNetwork<ProvingNetwork>) -> Amount {
    test.sdk()
        .stealth_outputs_api()
        .get_unspent_balance(&STEALTH_TARI_RESOURCE_ADDRESS)
        .unwrap()
        .balance
}

/// Scans once with the spend of `output` reported but not proven, and once more from where that scan read to.
async fn assert_unproven_spend_is_not_applied(
    network: &ProvingNetwork,
    test: &TestWithNetwork<ProvingNetwork>,
    output: &StealthOutputModel,
    reported_version: SubstateVersion,
) {
    report_spent(network, output, reported_version);
    let stats = scan(test).await;
    assert_eq!(stats.num_spent, 0);
    assert_eq!(stored_status(test, output), UNSPENT);
    assert_eq!(unspent_balance(test), Amount::from(VALUE));

    scan(test).await;
    let requests = network.utxo_stream_requests();
    assert!(
        requests.last().unwrap().contains(&(SHARD, SCANNED_TO)),
        "the scan moves past the unproven spend"
    );
    assert_eq!(stored_status(test, output), UNSPENT);
    assert_eq!(unspent_balance(test), Amount::from(VALUE));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reported_spend_without_a_proof_leaves_the_output_unspent() {
    let output = held_output(OutputStatus::Unspent);
    let (_, network, test) = wallet_holding(&output, IndexerTrust::Verify).await;
    network.serve_down_substate(substate_id(&output), VERSION, None);

    assert_unproven_spend_is_not_applied(&network, &test, &output, VERSION).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reported_spend_proven_by_an_untrusted_committee_leaves_the_output_unspent() {
    let output = held_output(OutputStatus::Unspent);
    let (_, network, test) = wallet_holding(&output, IndexerTrust::Verify).await;
    let impostors = ValidatorSetFixture::with_signers(NETWORK, 5, Signers::new(0xe0, 4), Signers::new(0xe1, 4));
    let id = substate_id(&output);
    let proof = substate_down_proof(&impostors, &impostors, &id, VERSION);
    network.serve_down_substate(id, VERSION, Some(proof));

    assert_unproven_spend_is_not_applied(&network, &test, &output, VERSION).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reported_spend_proven_for_another_version_leaves_the_output_unspent() {
    let output = held_output(OutputStatus::Unspent);
    let (sets, network, test) = wallet_holding(&output, IndexerTrust::Verify).await;
    let id = substate_id(&output);
    let proof = substate_down_proof(&sets[4], &sets[4], &id, VERSION);
    network.serve_down_substate(id, VERSION.next(), Some(proof));

    assert_unproven_spend_is_not_applied(&network, &test, &output, VERSION.next()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_proven_spend_marks_the_output_spent() {
    let output = held_output(OutputStatus::Unspent);
    let (sets, network, test) = wallet_holding(&output, IndexerTrust::Verify).await;
    let id = substate_id(&output);
    // Proven up in the anchor's epoch and down in the latest trusted one.
    let proof = substate_down_proof(&sets[1], &sets[4], &id, VERSION);
    network.serve_down_substate(id, VERSION, Some(proof));
    report_spent(&network, &output, VERSION);

    let stats = scan(&test).await;
    assert_eq!(stats.num_spent, 1);
    assert_eq!(stored_status(&test, &output), SPENT);
    assert_eq!(unspent_balance(&test), Amount::zero());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spend_whose_proof_could_not_be_fetched_is_applied_by_the_next_scan() {
    let output = held_output(OutputStatus::Unspent);
    let (sets, network, test) = wallet_holding(&output, IndexerTrust::Verify).await;
    let id = substate_id(&output);
    let proof = substate_down_proof(&sets[4], &sets[4], &id, VERSION);
    network.serve_down_substate(id, VERSION, Some(proof));
    report_spent(&network, &output, VERSION);
    network.fail_substate_queries(1);

    let err = try_scan(&test).await.unwrap_err();
    assert!(matches!(err, StealthScannerApiError::SubstateApiError(_)), "{err}");
    assert_eq!(stored_status(&test, &output), UNSPENT);

    let stats = scan(&test).await;
    assert_eq!(stats.num_spent, 1);
    assert_eq!(stored_status(&test, &output), SPENT);
    assert_eq!(unspent_balance(&test), Amount::zero());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reported_spend_of_an_output_already_spent_needs_no_proof() {
    // The network serves no substate, so a proof request would fail the scan.
    let output = held_output(OutputStatus::Spent);
    let (_, network, test) = wallet_holding(&output, IndexerTrust::Verify).await;
    report_spent(&network, &output, VERSION);

    let stats = scan(&test).await;
    assert_eq!(stats.num_spent, 0);
    assert_eq!(stored_status(&test, &output), SPENT);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trusting_wallet_applies_a_reported_spend_without_a_proof() {
    // The network serves no substate, so a proof request would fail the scan.
    let output = held_output(OutputStatus::Unspent);
    let (_, network, test) = wallet_holding(&output, IndexerTrust::Trust).await;
    report_spent(&network, &output, VERSION);

    let stats = scan(&test).await;
    assert_eq!(stats.num_spent, 1);
    assert_eq!(stored_status(&test, &output), SPENT);
    assert_eq!(unspent_balance(&test), Amount::zero());
}
