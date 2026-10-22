//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod support;

use std::sync::{Arc, Mutex};

use futures::StreamExt;
use ootle_byte_type::ToByteType;
use tari_crypto::{
    keys::{PublicKey, SecretKey},
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_engine_types::{
    Epoch,
    Utxo,
    UtxoOutput,
    crypto::{OutputBody, commit_amount},
    substate::SubstateId,
};
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
    models::{AccountWithAddress, StartOfShard, UtxoUnspent, UtxoUpdatePayload, WalletEvent, WalletUtxoUpdate},
    network::{
        FetchedSubstates,
        FetchedUtxos,
        SubstateQueryResult,
        TransactionFinalizedStream,
        TransactionQueryResult,
        UtxoUpdateStream,
        WalletNetworkInterface,
    },
    storage::{ReadableWalletStore, TagAndPublicNoncePair, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_wallet_sdk_services::{
    notify::Notify,
    utxo_scanner::{StealthUtxoScannerWorker, UtxoRecovery},
};
use tari_template_abi::TemplateDef;
use tari_template_lib::types::{
    Amount,
    ResourceAddress,
    TemplateAddress,
    UtxoAddress,
    UtxoId,
    constants::STEALTH_TARI_RESOURCE_ADDRESS,
    stealth::SpendAuthorization,
};

use crate::support::{TestWithNetwork, random_keypair};

#[derive(Debug, thiserror::Error)]
#[error("not found")]
struct FakeError;

impl IsNotFoundError for FakeError {
    fn is_not_found_error(&self) -> bool {
        true
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

#[derive(Debug, Default)]
struct UtxoNetworkState {
    /// The UTXOs streamed as unspent in shard 0 at state version 1.
    streamed: Vec<(UtxoId, Utxo)>,
    /// The UTXOs the full-UTXO lookup returns.
    indexed: Vec<(UtxoId, Utxo)>,
    /// The pairs the full-UTXO lookup reports spent.
    spent: Vec<TagAndPublicNoncePair>,
}

/// An indexer whose UTXO stream and full-UTXO lookup can disagree, as two indexers at different heights do.
#[derive(Debug, Clone, Default)]
struct UtxoNetwork {
    state: Arc<Mutex<UtxoNetworkState>>,
}

impl UtxoNetwork {
    fn stream(&self, utxo: (UtxoId, Utxo)) {
        self.state.lock().unwrap().streamed.push(utxo);
    }

    fn index(&self, utxo: (UtxoId, Utxo)) {
        self.state.lock().unwrap().indexed.push(utxo);
    }

    fn spend(&self, utxo: &Utxo) {
        self.state.lock().unwrap().spent.push(tag_and_nonce(utxo));
    }
}

fn tag_and_nonce(utxo: &Utxo) -> TagAndPublicNoncePair {
    let output = utxo.output().expect("an unspent UTXO has an output");
    (output.tag, output.output.public_nonce)
}

impl WalletNetworkInterface for UtxoNetwork {
    type Error = FakeError;

    async fn query_substate(
        &self,
        _address: &SubstateId,
        _version: Option<SubstateVersion>,
        _local_search_only: bool,
        _include_proof: bool,
    ) -> Result<SubstateQueryResult, Self::Error> {
        panic!("UtxoNetwork::query_substate called")
    }

    async fn get_substates(&self, _: Vec<SubstateId>, _: bool) -> Result<FetchedSubstates, Self::Error> {
        panic!("UtxoNetwork::get_substates called")
    }

    async fn submit_transaction(&self, _: Transaction) -> Result<TransactionId, Self::Error> {
        panic!("UtxoNetwork::submit_transaction called")
    }

    async fn submit_transaction_envelope(&self, _: TransactionEnvelope) -> Result<TransactionId, Self::Error> {
        panic!("UtxoNetwork::submit_transaction_envelope called")
    }

    async fn submit_dry_run_transaction(&self, _: Transaction) -> Result<TransactionQueryResult, Self::Error> {
        panic!("UtxoNetwork::submit_dry_run_transaction called")
    }

    async fn query_transaction_result(&self, _: TransactionId, _: bool) -> Result<TransactionQueryResult, Self::Error> {
        panic!("UtxoNetwork::query_transaction_result called")
    }

    async fn subscribe_transaction_finalized(&self) -> Result<TransactionFinalizedStream<Self::Error>, Self::Error> {
        Ok(futures::stream::pending().boxed())
    }

    async fn fetch_template_definition(&self, _: TemplateAddress) -> Result<TemplateDef, Self::Error> {
        panic!("UtxoNetwork::fetch_template_definition called")
    }

    async fn stream_stealth_utxo_updates(
        &self,
        _from_epoch: Epoch,
        _resource_address: ResourceAddress,
        shard_state_versions: Vec<(Shard, StateVersion)>,
        _unspent_only: bool,
    ) -> Result<UtxoUpdateStream<Self::Error>, Self::Error> {
        let shard = Shard::from(0u32);
        let synced = shard_state_versions
            .iter()
            .any(|(s, v)| *s == shard && *v >= StateVersion::from(1));
        if synced {
            return Ok(futures::stream::empty().boxed());
        }
        let sos = StartOfShard {
            shard,
            max_state_version: StateVersion::from(1),
            has_more: false,
        };
        let payloads = self
            .state
            .lock()
            .unwrap()
            .streamed
            .iter()
            .enumerate()
            .map(|(i, (_, utxo))| {
                let (tag, public_nonce) = tag_and_nonce(utxo);
                Ok(UtxoUpdatePayload {
                    sos: (i == 0).then_some(sos.clone()),
                    update: Some(WalletUtxoUpdate::Unspent(UtxoUnspent { tag, public_nonce })),
                    eos: None,
                })
            })
            .collect::<Vec<_>>();
        Ok(futures::stream::iter(payloads).boxed())
    }

    async fn list_watched_substates(
        &self,
        _: Option<TemplateAddress>,
        _: Option<u64>,
        _: Option<u64>,
    ) -> Result<Vec<WatchedSubstateItem>, Self::Error> {
        panic!("UtxoNetwork::list_watched_substates called")
    }

    async fn fetch_utxos(
        &self,
        _resource_address: ResourceAddress,
        tag_and_nonce_pairs: Vec<TagAndPublicNoncePair>,
    ) -> Result<FetchedUtxos, Self::Error> {
        let state = self.state.lock().unwrap();
        Ok(FetchedUtxos {
            unspent: state
                .indexed
                .iter()
                .filter(|(_, utxo)| tag_and_nonce_pairs.contains(&tag_and_nonce(utxo)))
                .cloned()
                .collect(),
            spent: state
                .spent
                .iter()
                .filter(|pair| tag_and_nonce_pairs.contains(pair))
                .copied()
                .collect(),
        })
    }

    async fn get_latest_epoch_checkpoint(&self, _shard: Shard) -> Result<EpochCheckpointEntry, Self::Error> {
        panic!("UtxoNetwork::get_latest_epoch_checkpoint called")
    }

    async fn list_epoch_checkpoints(
        &self,
        _from_epoch: Epoch,
        _limit: u32,
        _shard: Shard,
    ) -> Result<Vec<EpochCheckpointEntry>, Self::Error> {
        panic!("UtxoNetwork::list_epoch_checkpoints called")
    }

    async fn get_committee(&self, _epoch: Epoch, _shard: Shard) -> Result<ValidatorSetCommittee, Self::Error> {
        panic!("UtxoNetwork::get_committee called")
    }

    async fn get_current_epoch(&self) -> Result<Epoch, Self::Error> {
        panic!("UtxoNetwork::get_current_epoch called")
    }

    async fn get_consensus_epoch(&self) -> Result<Option<Epoch>, Self::Error> {
        panic!("UtxoNetwork::get_consensus_epoch called")
    }

    async fn wait_until_ready(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}

type Test = TestWithNetwork<UtxoNetwork>;

fn new_test(network: UtxoNetwork) -> (Test, AccountWithAddress) {
    let test = TestWithNetwork::without_accounts(network);
    let address = test.sdk().key_manager_api().next_account_address().unwrap();
    let account = test
        .sdk()
        .accounts_api()
        .create_account(Some("test"), false, address)
        .unwrap();
    (test, account)
}

/// A UTXO of `amount` that `account` can read and spend.
fn owned_utxo(test: &Test, account: &AccountWithAddress, amount: u64) -> (UtxoId, Utxo) {
    let sdk = test.sdk();
    let view_key = sdk.key_manager_api().get_key(account.view_only_key_id()).unwrap();
    let owner_key = sdk
        .key_manager_api()
        .get_key(account.owner_key_id().expect("account is locally owned"))
        .unwrap();
    let owner_public_key = RistrettoPublicKey::from_secret_key(&owner_key.secret);
    let view_public_key = RistrettoPublicKey::from_secret_key(&view_key.secret);
    let (nonce_secret, public_nonce) = random_keypair();

    let tag = sdk.stealth_crypto_api().derive_stealth_output_tag(
        sdk.network(),
        &view_key.secret,
        &public_nonce,
        &STEALTH_TARI_RESOURCE_ADDRESS,
    );
    let mask = RistrettoSecretKey::random(&mut rand::rng());
    let commitment = commit_amount(&mask, amount.into()).unwrap().to_byte_type();
    let encrypted_data = sdk
        .stealth_crypto_api()
        .encrypt_value_and_mask(amount, &mask, &view_public_key, &nonce_secret, None)
        .unwrap();
    let stealth_owner_public_key = sdk
        .stealth_crypto_api()
        .derive_stealth_owner_public_key(sdk.network(), &owner_public_key, &nonce_secret)
        .to_byte_type();

    let utxo = Utxo::new(UtxoOutput {
        output: OutputBody {
            public_nonce: public_nonce.to_byte_type(),
            encrypted_data,
            minimum_value_promise: 0,
            viewable_balance: None,
        },
        auth: SpendAuthorization::Key(stealth_owner_public_key),
        tag,
    });
    (UtxoId::from(commitment), utxo)
}

fn queued(test: &Test) -> usize {
    test.store()
        .with_read_tx(|tx| tx.utxo_process_queue_fetch_batch(None, 1000))
        .unwrap()
        .len()
}

fn is_stored(test: &Test, id: UtxoId) -> bool {
    test.sdk()
        .stealth_outputs_api()
        .utxo_exists(&UtxoAddress::new(STEALTH_TARI_RESOURCE_ADDRESS, id))
        .unwrap()
}

fn balance(test: &Test, account: &AccountWithAddress) -> Amount {
    test.sdk()
        .stealth_outputs_api()
        .get_unspent_balance_for_account_resource(account.account.component_address(), &STEALTH_TARI_RESOURCE_ADDRESS)
        .unwrap()
}

/// Scans the account once, queueing every streamed UTXO for recovery.
async fn scan(test: &Test, account: &AccountWithAddress) {
    let (_join, scanner) = StealthUtxoScannerWorker::new(test.sdk().clone(), Notify::new(10)).spawn();
    scanner
        .scan(*account.account.component_address(), STEALTH_TARI_RESOURCE_ADDRESS)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_utxo_the_lookup_does_not_know_stays_queued_until_it_is_recovered() {
    let network = UtxoNetwork::default();
    let (test, account) = new_test(network.clone());
    let (id, utxo) = owned_utxo(&test, &account, 1_234);
    network.stream((id, utxo.clone()));

    scan(&test, &account).await;
    assert_eq!(queued(&test), 1);

    let notify = Notify::new(10);
    let mut events = notify.subscribe();
    let mut recovery = UtxoRecovery::new(test.sdk().clone()).with_notify(notify);
    let num_unresolved = recovery.process_utxo_validation_queue().await.unwrap();
    assert_eq!(num_unresolved, 1);
    assert_eq!(
        queued(&test),
        1,
        "an output the lookup does not know is not known to be spent"
    );
    assert!(!is_stored(&test, id));
    assert!(
        events.try_recv().is_err(),
        "a round that resolves nothing publishes no events"
    );

    network.index((id, utxo));
    let num_unresolved = recovery.process_utxo_validation_queue().await.unwrap();
    assert_eq!(num_unresolved, 0);
    assert_eq!(queued(&test), 0);
    assert!(is_stored(&test, id));
    assert_eq!(balance(&test, &account), Amount::from_u64(1_234));

    let events = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert!(matches!(events.first(), Some(WalletEvent::UtxoRecoveryStarted(_))));
    assert!(matches!(events.last(), Some(WalletEvent::UtxoRecoveryCompleted(e)) if e.num_recovered == 1));
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, WalletEvent::UtxoRecoveryCompleted(_)))
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_partial_lookup_resolves_the_returned_and_spent_utxos_and_keeps_the_rest_queued() {
    let network = UtxoNetwork::default();
    let (test, account) = new_test(network.clone());
    let (returned_id, returned) = owned_utxo(&test, &account, 100);
    let (spent_id, spent) = owned_utxo(&test, &account, 200);
    let (unknown_id, unknown) = owned_utxo(&test, &account, 300);
    network.stream((returned_id, returned.clone()));
    network.stream((spent_id, spent.clone()));
    network.stream((unknown_id, unknown));
    network.index((returned_id, returned));
    network.spend(&spent);

    scan(&test, &account).await;
    assert_eq!(queued(&test), 3);

    let num_unresolved = UtxoRecovery::new(test.sdk().clone())
        .process_utxo_validation_queue()
        .await
        .unwrap();
    assert_eq!(num_unresolved, 1);
    assert_eq!(queued(&test), 1, "only the unknown output stays queued");
    assert!(is_stored(&test, returned_id));
    assert!(!is_stored(&test, spent_id));
    assert!(!is_stored(&test, unknown_id));
    assert_eq!(balance(&test, &account), Amount::from_u64(100));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_round_reaches_entries_queued_behind_more_than_a_batch_of_unknown_utxos() {
    let network = UtxoNetwork::default();
    let (test, account) = new_test(network.clone());
    let account_address = *account.account.component_address();
    let unknown = (0..150).map(|_| owned_utxo(&test, &account, 1)).collect::<Vec<_>>();
    let (id, utxo) = owned_utxo(&test, &account, 777);
    network.index((id, utxo.clone()));

    let queue_items = unknown
        .iter()
        .chain([&(id, utxo)])
        .map(|(_, utxo)| {
            let (tag, public_nonce) = tag_and_nonce(utxo);
            (account_address, UtxoUnspent { tag, public_nonce })
        })
        .collect::<Vec<_>>();
    test.store()
        .with_write_tx(|tx| tx.utxo_process_queue_extend(&STEALTH_TARI_RESOURCE_ADDRESS, queue_items))
        .unwrap();
    assert_eq!(queued(&test), 151);

    let num_unresolved = UtxoRecovery::new(test.sdk().clone())
        .process_utxo_validation_queue()
        .await
        .unwrap();
    assert_eq!(num_unresolved, 150);
    assert_eq!(queued(&test), 150);
    assert!(
        is_stored(&test, id),
        "the entry behind the unknown ones is processed in the same round"
    );
}
