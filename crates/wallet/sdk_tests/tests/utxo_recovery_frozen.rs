//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! A wallet that verifies its indexer holds a recovered stealth output as frozen only once the indexer proves that the
//! network holds the output frozen.

mod support;

use tari_engine_types::{
    Utxo,
    substate::{SubstateId, SubstateValue},
};
use tari_ootle_common_types::{Epoch, SubstateVersion};
use tari_ootle_light_client::test_fixtures::{NETWORK, Signers, ValidatorSetFixture, substate_proof};
use tari_ootle_wallet_sdk::{
    IndexerTrust,
    models::{AccountWithAddress, StealthOutputModel, UtxoUnspent},
    storage::{WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_wallet_sdk_services::utxo_scanner::UtxoRecovery;
use tari_template_lib::types::{UtxoAddress, UtxoId, constants::STEALTH_TARI_RESOURCE_ADDRESS};

use crate::support::{ProvingNetwork, TestWithNetwork};

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

/// The wallet recovers `utxo` as not frozen, then recovers it again with the indexer reporting it frozen.
async fn assert_unproven_freeze_is_not_applied(
    network: &ProvingNetwork,
    test: &Test,
    account: &AccountWithAddress,
    id: UtxoId,
    utxo: &Utxo,
) {
    assert_eq!(recover(test, network, account, id, utxo).await, 0);
    assert!(!stored(test, id).is_frozen);

    assert_eq!(recover(test, network, account, id, &frozen(utxo)).await, 0);
    assert!(
        !stored(test, id).is_frozen,
        "a held output stays spendable without a proof that it is frozen"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reported_freeze_without_a_proof_leaves_a_held_output_spendable() {
    let (_, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    network.serve_substate(substate_id(id), VERSION, SubstateValue::Utxo(frozen(&utxo)), None);

    assert_unproven_freeze_is_not_applied(&network, &test, &account, id, &utxo).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reported_freeze_proven_by_an_untrusted_committee_leaves_a_held_output_spendable() {
    let (_, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    let impostors = ValidatorSetFixture::with_signers(NETWORK, 5, Signers::new(0xe0, 4), Signers::new(0xe1, 4));
    let value = SubstateValue::Utxo(frozen(&utxo));
    let proof = substate_proof(&impostors, &substate_id(id), VERSION, &value);
    network.serve_substate(substate_id(id), VERSION, value, Some(proof));

    assert_unproven_freeze_is_not_applied(&network, &test, &account, id, &utxo).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reported_freeze_proven_unfrozen_leaves_a_held_output_spendable() {
    let (sets, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    let value = SubstateValue::Utxo(utxo.clone());
    let proof = substate_proof(&sets[4], &substate_id(id), VERSION, &value);
    network.serve_substate(substate_id(id), VERSION, value, Some(proof));

    assert_unproven_freeze_is_not_applied(&network, &test, &account, id, &utxo).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_output_reported_frozen_without_a_proof_is_stored_spendable() {
    let (_, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    network.serve_substate(substate_id(id), VERSION, SubstateValue::Utxo(frozen(&utxo)), None);

    assert_eq!(recover(&test, &network, &account, id, &frozen(&utxo)).await, 0);
    let output = stored(&test, id);
    assert!(!output.is_frozen);
    assert_eq!(output.value, VALUE);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_proven_freeze_freezes_the_output() {
    let (sets, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    let value = SubstateValue::Utxo(frozen(&utxo));
    let proof = substate_proof(&sets[4], &substate_id(id), VERSION, &value);
    network.serve_substate(substate_id(id), VERSION, value, Some(proof));

    assert_eq!(recover(&test, &network, &account, id, &utxo).await, 0);
    assert!(!stored(&test, id).is_frozen);
    assert_eq!(recover(&test, &network, &account, id, &frozen(&utxo)).await, 0);
    assert!(stored(&test, id).is_frozen);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_freeze_whose_proof_could_not_be_fetched_stays_queued_until_it_is_proven() {
    let (sets, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    let value = SubstateValue::Utxo(frozen(&utxo));
    let proof = substate_proof(&sets[4], &substate_id(id), VERSION, &value);
    network.serve_substate(substate_id(id), VERSION, value, Some(proof));
    network.fail_substate_queries(1);

    assert_eq!(recover(&test, &network, &account, id, &frozen(&utxo)).await, 1);
    assert!(
        !test
            .sdk()
            .stealth_outputs_api()
            .utxo_exists(&UtxoAddress::new(STEALTH_TARI_RESOURCE_ADDRESS, id))
            .unwrap()
    );

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
    let value = SubstateValue::Utxo(frozen(&utxo));
    let proof = substate_proof(&sets[4], &substate_id(id), VERSION, &value);
    network.serve_substate(substate_id(id), VERSION, value, Some(proof));
    assert_eq!(recover(&test, &network, &account, id, &frozen(&utxo)).await, 0);
    assert!(stored(&test, id).is_frozen);

    let queries = network.substate_queries();
    assert_eq!(recover(&test, &network, &account, id, &utxo).await, 0);
    assert!(!stored(&test, id).is_frozen);
    assert_eq!(network.substate_queries(), queries);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reported_freeze_of_an_output_held_frozen_needs_no_proof() {
    let (sets, network, test, account) = wallet(IndexerTrust::Verify).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);
    let value = SubstateValue::Utxo(frozen(&utxo));
    let proof = substate_proof(&sets[4], &substate_id(id), VERSION, &value);
    network.serve_substate(substate_id(id), VERSION, value, Some(proof));
    assert_eq!(recover(&test, &network, &account, id, &frozen(&utxo)).await, 0);

    let queries = network.substate_queries();
    assert_eq!(recover(&test, &network, &account, id, &frozen(&utxo)).await, 0);
    assert!(stored(&test, id).is_frozen);
    assert_eq!(network.substate_queries(), queries);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trusting_wallet_applies_a_reported_freeze_without_a_proof() {
    // The network serves no substate, so a proof request would panic.
    let (_, network, test, account) = wallet(IndexerTrust::Trust).await;
    let (id, utxo) = test.owned_utxo(&account, VALUE);

    assert_eq!(recover(&test, &network, &account, id, &utxo).await, 0);
    assert_eq!(recover(&test, &network, &account, id, &frozen(&utxo)).await, 0);
    assert!(stored(&test, id).is_frozen);
}
