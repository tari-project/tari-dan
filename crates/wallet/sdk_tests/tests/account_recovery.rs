//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod support;

use std::{
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures::StreamExt;
use ootle_byte_type::ToByteType;
use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey};
use tari_engine_types::{Epoch, Utxo, substate::SubstateId};
use tari_indexer_client::types::{EpochCheckpointEntry, ValidatorSetCommittee, WatchedSubstateItem};
use tari_ootle_common_types::{
    StateVersion,
    SubstateVersion,
    optional::IsNotFoundError,
    response_status::{ResponseErrorStatus, TransactionStatusResponseError},
    shard::Shard,
};
use tari_ootle_transaction::{Transaction, TransactionEnvelope, TransactionId};
use tari_ootle_wallet_sdk::{
    apis::config::ConfigKey,
    models::{KeyBranch, KeyId, StartOfShard, UtxoUnspent, UtxoUpdatePayload, WalletUtxoUpdate},
    network::{
        FetchedSubstates,
        SubstateQueryResult,
        TransactionFinalizedStream,
        TransactionQueryResult,
        UtxoUpdateStream,
        WalletNetworkInterface,
    },
    storage::TagAndPublicNoncePair,
};
use tari_ootle_wallet_sdk_services::{
    account_monitor::AccountMonitor,
    account_recovery::AccountRecoveryService,
    notify::Notify,
    utxo_scanner::StealthUtxoScannerWorker,
};
use tari_shutdown::Shutdown;
use tari_template_abi::TemplateDef;
use tari_template_lib::types::{
    ComponentAddress,
    ResourceAddress,
    TemplateAddress,
    UtxoId,
    constants::STEALTH_TARI_RESOURCE_ADDRESS,
    crypto::UtxoTag,
};

use crate::support::{TestWithNetwork, random_keypair};

#[derive(Debug, thiserror::Error)]
enum FakeError {
    #[error("not found")]
    NotFound,
    #[error("unavailable")]
    Unavailable,
}

impl IsNotFoundError for FakeError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::NotFound)
    }
}

impl TransactionStatusResponseError for FakeError {
    fn get_status(&self) -> ResponseErrorStatus {
        ResponseErrorStatus::NotFound {
            message: self.to_string(),
        }
    }

    fn get_error_message(&self) -> String {
        self.to_string()
    }
}

/// A network with no accounts on chain and at most one stealth UTXO, in shard 0.
#[derive(Debug, Clone, Default)]
struct OneUtxoNetwork {
    utxo: Arc<Mutex<Option<UtxoUnspent>>>,
    failing_substate_queries: Arc<AtomicUsize>,
}

impl OneUtxoNetwork {
    fn set_utxo(&self, utxo: UtxoUnspent) {
        *self.utxo.lock().unwrap() = Some(utxo);
    }

    /// Makes the next `count` substate queries fail as if the indexer were unavailable.
    fn fail_substate_queries(&self, count: usize) {
        self.failing_substate_queries.store(count, Ordering::SeqCst);
    }
}

impl WalletNetworkInterface for OneUtxoNetwork {
    type Error = FakeError;

    async fn query_substate(
        &self,
        _address: &SubstateId,
        _version: Option<SubstateVersion>,
        _local_search_only: bool,
        _include_proof: bool,
    ) -> Result<SubstateQueryResult, Self::Error> {
        let failed = self
            .failing_substate_queries
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok();
        if failed {
            return Err(FakeError::Unavailable);
        }
        Err(FakeError::NotFound)
    }

    async fn get_substates(&self, _: Vec<SubstateId>, _: bool) -> Result<FetchedSubstates, Self::Error> {
        panic!("OneUtxoNetwork::get_substates called")
    }

    async fn submit_transaction(&self, _: Transaction) -> Result<TransactionId, Self::Error> {
        panic!("OneUtxoNetwork::submit_transaction called")
    }

    async fn submit_transaction_envelope(&self, _: TransactionEnvelope) -> Result<TransactionId, Self::Error> {
        panic!("OneUtxoNetwork::submit_transaction_envelope called")
    }

    async fn submit_dry_run_transaction(&self, _: Transaction) -> Result<TransactionQueryResult, Self::Error> {
        panic!("OneUtxoNetwork::submit_dry_run_transaction called")
    }

    async fn query_transaction_result(&self, _: TransactionId, _: bool) -> Result<TransactionQueryResult, Self::Error> {
        panic!("OneUtxoNetwork::query_transaction_result called")
    }

    async fn subscribe_transaction_finalized(&self) -> Result<TransactionFinalizedStream<Self::Error>, Self::Error> {
        Ok(futures::stream::pending().boxed())
    }

    async fn fetch_template_definition(&self, _: TemplateAddress) -> Result<TemplateDef, Self::Error> {
        panic!("OneUtxoNetwork::fetch_template_definition called")
    }

    async fn stream_stealth_utxo_updates(
        &self,
        _from_epoch: Epoch,
        _resource_address: ResourceAddress,
        shard_state_versions: Vec<(Shard, StateVersion)>,
        _unspent_only: bool,
    ) -> Result<UtxoUpdateStream<Self::Error>, Self::Error> {
        let shard = Shard::from(0u32);
        let already_synced = shard_state_versions
            .iter()
            .any(|(s, v)| *s == shard && *v >= StateVersion::from(1));
        let Some(utxo) = self.utxo.lock().unwrap().clone().filter(|_| !already_synced) else {
            return Ok(futures::stream::empty().boxed());
        };
        let payload = UtxoUpdatePayload {
            sos: Some(StartOfShard {
                shard,
                max_state_version: StateVersion::from(1),
                has_more: false,
            }),
            update: Some(WalletUtxoUpdate::Unspent(utxo)),
            eos: None,
        };
        Ok(futures::stream::iter([Ok(payload)]).boxed())
    }

    async fn list_watched_substates(
        &self,
        _: Option<TemplateAddress>,
        _: Option<u64>,
        _: Option<u64>,
    ) -> Result<Vec<WatchedSubstateItem>, Self::Error> {
        panic!("OneUtxoNetwork::list_watched_substates called")
    }

    async fn get_unspent_utxos(
        &self,
        _: ResourceAddress,
        _: Vec<TagAndPublicNoncePair>,
    ) -> Result<Vec<(UtxoId, Utxo)>, Self::Error> {
        panic!("OneUtxoNetwork::get_unspent_utxos called")
    }

    async fn get_latest_epoch_checkpoint(&self, _shard: Shard) -> Result<EpochCheckpointEntry, Self::Error> {
        panic!("OneUtxoNetwork called")
    }

    async fn list_epoch_checkpoints(
        &self,
        _from_epoch: Epoch,
        _limit: u32,
        _shard: Shard,
    ) -> Result<Vec<EpochCheckpointEntry>, Self::Error> {
        panic!("OneUtxoNetwork called")
    }

    async fn get_committee(&self, _epoch: Epoch, _shard: Shard) -> Result<ValidatorSetCommittee, Self::Error> {
        panic!("OneUtxoNetwork called")
    }

    async fn get_current_epoch(&self) -> Result<Epoch, Self::Error> {
        panic!("OneUtxoNetwork::get_current_epoch called")
    }

    async fn wait_until_ready(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Builds a UTXO that the view key at `key_index` recognises as its own.
fn utxo_for_key_index(test: &TestWithNetwork<impl WalletNetworkInterface>, key_index: u64) -> UtxoUnspent {
    let sdk = test.sdk();
    let view_key = sdk
        .key_manager_api()
        .get_key(KeyId::derived(KeyBranch::ViewOnlyKey, key_index))
        .unwrap();
    let (_, public_nonce) = random_keypair();
    let tag: UtxoTag = sdk.stealth_crypto_api().derive_stealth_output_tag(
        sdk.network(),
        &view_key.secret,
        &public_nonce,
        &STEALTH_TARI_RESOURCE_ADDRESS,
    );
    UtxoUnspent {
        tag,
        public_nonce: public_nonce.to_byte_type(),
    }
}

fn account_address_for_key_index(
    test: &TestWithNetwork<impl WalletNetworkInterface>,
    key_index: u64,
) -> ComponentAddress {
    let key = test.sdk().key_manager_api().derive_account_key(key_index).unwrap();
    let public_key = RistrettoPublicKey::from_secret_key(&key.key).to_byte_type();
    test.sdk()
        .accounts_api()
        .derive_account_address_from_public_key(&public_key)
}

const ABANDON_AFTER_NOT_FOUND: usize = 3;

/// Runs recovery against a network where key index 1 holds the only UTXO.
async fn run_recovery(test: &TestWithNetwork<OneUtxoNetwork>) {
    let sdk = test.sdk().clone();
    let shutdown = Shutdown::new();
    let notify = Notify::new(100);
    let (_scanner_join, utxo_scanner_handle) = StealthUtxoScannerWorker::new(sdk.clone(), notify.clone()).spawn();
    let (account_monitor, account_monitor_handle) =
        AccountMonitor::new(notify, sdk.clone(), utxo_scanner_handle.clone(), shutdown.to_signal());
    tokio::spawn(account_monitor.run());

    let seed_birthday = sdk.key_manager_api().get_cipher_seed_birthday_epoch().unwrap();
    let recovery = AccountRecoveryService::new(
        sdk,
        account_monitor_handle,
        utxo_scanner_handle,
        ABANDON_AFTER_NOT_FOUND,
        seed_birthday,
    );
    tokio::time::timeout(Duration::from_secs(120), recovery.scan())
        .await
        .expect("recovery did not finish");
}

fn new_test() -> (TestWithNetwork<OneUtxoNetwork>, OneUtxoNetwork) {
    let network = OneUtxoNetwork::default();
    let test = TestWithNetwork::without_accounts(network.clone());
    network.set_utxo(utxo_for_key_index(&test, 1));
    (test, network)
}

/// Key 0 is unused, key 1 holds a UTXO, and keys 2..=4 are the unused keys that end the scan.
fn assert_recovered_only_key_1(test: &TestWithNetwork<OneUtxoNetwork>) {
    let sdk = test.sdk();
    let accounts_api = sdk.accounts_api();
    for index in [0, 2, 3, 4] {
        assert!(
            !accounts_api
                .exists_by_address(&account_address_for_key_index(test, index))
                .unwrap(),
            "unused account at key index {index} was not removed"
        );
    }
    let recovered = accounts_api
        .get_account_by_address(&account_address_for_key_index(test, 1))
        .unwrap();
    assert!(
        recovered.account.is_default(),
        "the default moves off the removed key 0 account"
    );
    assert!(!sdk.config_api().get::<bool>(ConfigKey::RecoveryNeeded).unwrap());
    assert_eq!(
        sdk.config_api()
            .get::<u64>(ConfigKey::RecoveryMaxProbedKeyIndex)
            .unwrap(),
        4
    );

    // The next account reuses a removed key index, so it scans from the seed birthday.
    let address = sdk.key_manager_api().next_account_address().unwrap();
    assert_eq!(address.owner_key_id.derived_index(), Some(2));
    let created = accounts_api.create_account(Some("new"), false, address).unwrap();
    assert_eq!(
        created.account.birthday_epoch,
        sdk.key_manager_api().get_cipher_seed_birthday_epoch().unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn recovery_keeps_stealth_only_accounts_and_removes_unused_ones() {
    let (test, _network) = new_test();
    run_recovery(&test).await;
    assert_recovered_only_key_1(&test);
}

#[tokio::test(flavor = "multi_thread")]
async fn recovery_redoes_an_interrupted_run() {
    let (test, _network) = new_test();
    let sdk = test.sdk();
    // An interrupted run leaves rows for the keys it reached and the key index advanced past them.
    for index in 0..3 {
        let key = sdk.key_manager_api().next_key(KeyBranch::Account).unwrap();
        assert_eq!(key.key_index(), index);
        sdk.accounts_api()
            .add_account(
                Some(&format!("recovered-account-{index}")),
                &account_address_for_key_index(&test, index),
                KeyId::derived(KeyBranch::ViewOnlyKey, index),
                key.as_key_id(),
                Epoch::zero(),
                false,
                index == 0,
            )
            .unwrap();
    }

    run_recovery(&test).await;
    assert_recovered_only_key_1(&test);
}

#[tokio::test(flavor = "multi_thread")]
async fn recovery_retries_a_key_that_fails_to_scan() {
    let (test, network) = new_test();
    // The first query is for key 0.
    network.fail_substate_queries(1);
    run_recovery(&test).await;
    assert_recovered_only_key_1(&test);
}

#[tokio::test(flavor = "multi_thread")]
async fn utxo_scanner_answers_every_waiting_scan_beyond_its_concurrency_limit() {
    const NUM_ACCOUNTS: u64 = 25;

    let test = TestWithNetwork::with_network(OneUtxoNetwork::default());
    let sdk = test.sdk().clone();
    let (_scanner_join, utxo_scanner_handle) = StealthUtxoScannerWorker::new(sdk.clone(), Notify::new(100)).spawn();

    let mut accounts = Vec::new();
    for index in 1..=NUM_ACCOUNTS {
        let address = sdk.key_manager_api().derive_account_address(index).unwrap();
        let account = sdk
            .accounts_api()
            .create_account(Some(&format!("account-{index}")), false, address)
            .unwrap();
        accounts.push(*account.account.component_address());
    }

    let scans = accounts.iter().flat_map(|account| {
        // A waiting scan for an account whose scan is already running must still be answered.
        [
            utxo_scanner_handle.scan(*account, STEALTH_TARI_RESOURCE_ADDRESS),
            utxo_scanner_handle.scan(*account, STEALTH_TARI_RESOURCE_ADDRESS),
        ]
    });
    let results = tokio::time::timeout(Duration::from_secs(30), futures::future::join_all(scans))
        .await
        .expect("a waiting scan was never answered");
    for result in results {
        result.unwrap();
    }
}
