//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! A wallet that verifies its indexer recovers a stealth output it does not hold only once the indexer proves that the
//! network holds the output, and stores it with the contents and frozen flag the proof shows. It holds a recovered
//! output as frozen only once the indexer proves that the network holds the output frozen.

mod support;

use ootle_byte_type::ToByteType;
use tari_engine_types::{
    Utxo,
    substate::{SubstateId, SubstateValue},
};
use tari_indexer_client::types::SubstateProof;
use tari_ootle_common_types::{Epoch, SubstateVersion};
use tari_ootle_light_client::test_fixtures::{NETWORK, Signers, ValidatorSetFixture, substate_proof};
use tari_ootle_wallet_sdk::{
    IndexerTrust,
    models::{AccountWithAddress, OutputStatus, StealthOutputModel, UtxoUnspent},
    storage::{ReadableWalletStore, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_wallet_sdk_services::utxo_scanner::UtxoRecovery;
use tari_template_lib::types::{
    Amount,
    UtxoAddress,
    UtxoId,
    constants::STEALTH_TARI_RESOURCE_ADDRESS,
    stealth::SpendAuthorization,
};

use crate::support::{ProvingNetwork, TestWithNetwork, random_keypair};

const VALUE: u64 = 1_234;
const VERSION: SubstateVersion = SubstateVersion::ZERO;

type Test = TestWithNetwork<ProvingNetwork>;

/// A wallet with one account, anchored on the validator set of epoch 2 and walked to that of epoch 5.
async fn wallet(indexer_trust: IndexerTrust) -> (Vec<ValidatorSetFixture>, ProvingNetwork, Test, AccountWithAddress) {
    let sets = (1..=5).map(ValidatorSetFixture::new).collect::<Vec<_>>();
    let network = ProvingNetwork::new(&sets, 0);
    let test = TestWithNetwork::without_accounts_trusting(network.clone(), indexer_trust);
    test.sdk().validator_set_api().sync().await.unwrap();
    network.set_latest(3);
    let trusted = test.sdk().validator_set_api().sync().await.unwrap();
    assert_eq!(trusted.epoch(), Epoch(5));
    let address = test.sdk().key_manager_api().next_account_address().unwrap();
    let account = test
        .sdk()
        .accounts_api()
        .create_account(Some("test"), false, address)
        .unwrap();
    (sets, network, test, account)
}

fn substate_id(id: UtxoId) -> SubstateId {
    SubstateId::Utxo(UtxoAddress::new(STEALTH_TARI_RESOURCE_ADDRESS, id))
}

fn frozen(utxo: &Utxo) -> Utxo {
    let mut utxo = utxo.clone();
    utxo.freeze();
    utxo
}

/// The indexer serves `id` holding `utxo`, proven by the committee of the latest trusted validator set.
fn serve_proven(sets: &[ValidatorSetFixture], network: &ProvingNetwork, id: UtxoId, utxo: &Utxo) {
    let value = SubstateValue::Utxo(utxo.clone());
    let proof = substate_proof(&sets[4], &substate_id(id), VERSION, &value);
    network.serve_substate(substate_id(id), VERSION, value, Some(proof));
}

/// The indexer returns `utxo` to the full-UTXO lookup, and the wallet recovers it.
async fn recover(
    test: &Test,
    network: &ProvingNetwork,
    account: &AccountWithAddress,
    id: UtxoId,
    utxo: &Utxo,
) -> usize {
    network.index_utxo(id, utxo.clone());
    let output = utxo.output().unwrap();
    let item = (*account.account.component_address(), UtxoUnspent {
        tag: output.tag,
        public_nonce: output.output.public_nonce,
    });
    test.store()
        .with_write_tx(|tx| tx.utxo_process_queue_extend(&STEALTH_TARI_RESOURCE_ADDRESS, [item]))
        .unwrap();
    UtxoRecovery::new(test.sdk().clone())
        .process_utxo_validation_queue()
        .await
        .unwrap()
}

fn stored(test: &Test, id: UtxoId) -> StealthOutputModel {
    test.sdk()
        .stealth_outputs_api()
        .utxos_get_many(&STEALTH_TARI_RESOURCE_ADDRESS, None, None)
        .unwrap()
        .into_iter()
        .find(|output| output.commitment == id.into_commitment_bytes())
        .expect("the output is stored")
}

fn is_stored(test: &Test, id: UtxoId) -> bool {
    test.sdk()
        .stealth_outputs_api()
        .utxo_exists(&UtxoAddress::new(STEALTH_TARI_RESOURCE_ADDRESS, id))
        .unwrap()
}

fn queued(test: &Test) -> usize {
    test.store()
        .with_read_tx(|tx| tx.utxo_process_queue_fetch_batch(None, 1000))
        .unwrap()
        .len()
}

fn balance(test: &Test, account: &AccountWithAddress) -> Amount {
    test.sdk()
        .stealth_outputs_api()
        .get_unspent_balance_for_account_resource(account.account.component_address(), &STEALTH_TARI_RESOURCE_ADDRESS)
        .unwrap()
}

/// The wallet recovers `utxo` and so holds it.
async fn hold(
    sets: &[ValidatorSetFixture],
    network: &ProvingNetwork,
    test: &Test,
    account: &AccountWithAddress,
    id: UtxoId,
    utxo: &Utxo,
) {
    serve_proven(sets, network, id, utxo);
    assert_eq!(recover(test, network, account, id, utxo).await, 0);
    assert_eq!(stored(test, id).is_frozen, utxo.is_frozen());
}

/// The lookup returns `utxo` for an output the wallet does not hold, and the wallet leaves it queued without storing
/// it.
async fn assert_not_recovered(
    network: &ProvingNetwork,
    test: &Test,
    account: &AccountWithAddress,
    id: UtxoId,
    utxo: &Utxo,
) {
    assert_eq!(recover(test, network, account, id, utxo).await, 1);
    assert!(!is_stored(test, id));
    assert_eq!(balance(test, account), Amount::from_u64(0));
    assert_eq!(queued(test), 1, "an output that is not proven stays queued");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_output_without_a_proof_is_not_stored() {
    let (_, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    network.serve_substate(substate_id(id), VERSION, SubstateValue::Utxo(utxo.clone()), None);

    assert_not_recovered(&network, &test, &account, id, &utxo).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_output_the_network_holds_down_is_not_stored() {
    let (_, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    network.serve_down_substate(substate_id(id), VERSION, None);

    assert_not_recovered(&network, &test, &account, id, &utxo).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_output_proven_by_an_untrusted_committee_is_not_stored() {
    let (_, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    let impostors = ValidatorSetFixture::with_signers(NETWORK, 5, Signers::new(0xe0, 4), Signers::new(0xe1, 4));
    let value = SubstateValue::Utxo(utxo.clone());
    let proof = substate_proof(&impostors, &substate_id(id), VERSION, &value);
    network.serve_substate(substate_id(id), VERSION, value, Some(proof));

    assert_not_recovered(&network, &test, &account, id, &utxo).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_output_proven_with_another_tag_and_nonce_is_not_stored() {
    let (sets, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    let (_, other) = test.owned_utxo(&account, VALUE);
    serve_proven(&sets, &network, id, &other);

    assert_not_recovered(&network, &test, &account, id, &utxo).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unproven_output_stays_queued_until_it_is_proven() {
    let (sets, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    network.serve_substate(substate_id(id), VERSION, SubstateValue::Utxo(utxo.clone()), None);
    assert_not_recovered(&network, &test, &account, id, &utxo).await;

    serve_proven(&sets, &network, id, &utxo);
    let num_unresolved = UtxoRecovery::new(test.sdk().clone())
        .process_utxo_validation_queue()
        .await
        .unwrap();
    assert_eq!(num_unresolved, 0);
    assert_eq!(queued(&test), 0);
    assert_eq!(stored(&test, id).value, VALUE);
    assert_eq!(balance(&test, &account), Amount::from_u64(VALUE));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_output_is_stored_with_the_frozen_flag_the_proof_shows() {
    let (sets, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (unfrozen_id, unfrozen) = test.owned_utxo(&account, VALUE);
    serve_proven(&sets, &network, unfrozen_id, &unfrozen);
    let (frozen_id, held_frozen) = test.owned_utxo(&account, VALUE);
    serve_proven(&sets, &network, frozen_id, &frozen(&held_frozen));

    assert_eq!(
        recover(&test, &network, &account, unfrozen_id, &frozen(&unfrozen)).await,
        0
    );
    let output = stored(&test, unfrozen_id);
    assert!(!output.is_frozen);
    assert_eq!(output.value, VALUE);

    assert_eq!(recover(&test, &network, &account, frozen_id, &held_frozen).await, 0);
    assert!(stored(&test, frozen_id).is_frozen);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_held_output_needs_no_proof_that_it_exists() {
    let (sets, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    hold(&sets, &network, &test, &account, id, &utxo).await;

    let queries = network.substate_queries();
    assert_eq!(recover(&test, &network, &account, id, &utxo).await, 0);
    assert_eq!(network.substate_queries(), queries);
    assert_eq!(queued(&test), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_held_output_keeps_its_status_when_the_lookup_reports_other_contents() {
    let (sets, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    hold(&sets, &network, &test, &account, id, &utxo).await;

    let mut output = utxo.output().unwrap().clone();
    output.auth = SpendAuthorization::Key(random_keypair().1.to_byte_type());
    let reported = Utxo::new(output);
    assert_eq!(recover(&test, &network, &account, id, &reported).await, 0);
    assert_eq!(stored(&test, id).status, OutputStatus::Unspent);
    assert_eq!(balance(&test, &account), Amount::from_u64(VALUE));
}

/// The wallet holds `utxo` as not frozen, then recovers it again with the indexer reporting it frozen, serving
/// `value` with `proof`.
async fn assert_unproven_freeze_is_not_applied(
    sets: &[ValidatorSetFixture],
    network: &ProvingNetwork,
    test: &Test,
    account: &AccountWithAddress,
    id: UtxoId,
    utxo: &Utxo,
    value: SubstateValue,
    proof: Option<SubstateProof>,
) {
    hold(sets, network, test, account, id, utxo).await;
    network.serve_substate(substate_id(id), VERSION, value, proof);

    assert_eq!(recover(test, network, account, id, &frozen(utxo)).await, 0);
    assert!(
        !stored(test, id).is_frozen,
        "a held output stays spendable without a proof that it is frozen"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reported_freeze_without_a_proof_leaves_a_held_output_spendable() {
    let (sets, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    let value = SubstateValue::Utxo(frozen(&utxo));

    assert_unproven_freeze_is_not_applied(&sets, &network, &test, &account, id, &utxo, value, None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reported_freeze_proven_by_an_untrusted_committee_leaves_a_held_output_spendable() {
    let (sets, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    let impostors = ValidatorSetFixture::with_signers(NETWORK, 5, Signers::new(0xe0, 4), Signers::new(0xe1, 4));
    let value = SubstateValue::Utxo(frozen(&utxo));
    let proof = substate_proof(&impostors, &substate_id(id), VERSION, &value);

    assert_unproven_freeze_is_not_applied(&sets, &network, &test, &account, id, &utxo, value, Some(proof)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reported_freeze_proven_unfrozen_leaves_a_held_output_spendable() {
    let (sets, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    let value = SubstateValue::Utxo(utxo.clone());
    let proof = substate_proof(&sets[4], &substate_id(id), VERSION, &value);

    assert_unproven_freeze_is_not_applied(&sets, &network, &test, &account, id, &utxo, value, Some(proof)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_proven_freeze_freezes_a_held_output() {
    let (sets, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    hold(&sets, &network, &test, &account, id, &utxo).await;

    serve_proven(&sets, &network, id, &frozen(&utxo));
    assert_eq!(recover(&test, &network, &account, id, &frozen(&utxo)).await, 0);
    assert!(stored(&test, id).is_frozen);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_output_whose_proof_could_not_be_fetched_stays_queued_until_it_is_proven() {
    let (sets, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    serve_proven(&sets, &network, id, &frozen(&utxo));
    network.fail_substate_queries(1);

    assert_eq!(recover(&test, &network, &account, id, &frozen(&utxo)).await, 1);
    assert!(!is_stored(&test, id));

    let num_unresolved = UtxoRecovery::new(test.sdk().clone())
        .process_utxo_validation_queue()
        .await
        .unwrap();
    assert_eq!(num_unresolved, 0);
    assert!(stored(&test, id).is_frozen);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reported_unfreeze_needs_no_proof() {
    let (sets, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    hold(&sets, &network, &test, &account, id, &frozen(&utxo)).await;

    let queries = network.substate_queries();
    assert_eq!(recover(&test, &network, &account, id, &utxo).await, 0);
    assert!(!stored(&test, id).is_frozen);
    assert_eq!(network.substate_queries(), queries);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reported_freeze_of_an_output_held_frozen_needs_no_proof() {
    let (sets, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    hold(&sets, &network, &test, &account, id, &frozen(&utxo)).await;

    let queries = network.substate_queries();
    assert_eq!(recover(&test, &network, &account, id, &frozen(&utxo)).await, 0);
    assert!(stored(&test, id).is_frozen);
    assert_eq!(network.substate_queries(), queries);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trusting_wallet_recovers_and_freezes_an_output_without_a_proof() {
    // The network serves no substate, so a proof request would panic.
    let (_, network, test, account) = wallet(IndexerTrust::Trust).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);

    assert_eq!(recover(&test, &network, &account, id, &utxo).await, 0);
    assert_eq!(recover(&test, &network, &account, id, &frozen(&utxo)).await, 0);
    assert!(stored(&test, id).is_frozen);
}
