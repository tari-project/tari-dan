//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! The network can return a substate value that is not the type of substate its id addresses. The wallet must refuse
//! such a pair with an error and must not act on it.

mod support;

use std::time::Duration;

use futures::StreamExt;
use tari_consensus_types::Decision;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine_types::{
    Epoch,
    Utxo,
    commit_result::{ExecuteResult, FinalizeResult, TransactionResult},
    component::{Component, ComponentBody, ComponentHeader},
    fees::FeeReceipt,
    resource_container::ResourceContainer,
    substate::{Substate, SubstateDiff, SubstateId, SubstateValue},
    vault::Vault,
};
use tari_indexer_client::types::{EpochCheckpointEntry, ValidatorSetCommittee, WatchedSubstateItem};
use tari_ootle_common_types::{StateVersion, SubstateVersion, shard::Shard};
use tari_ootle_transaction::{Transaction, TransactionEnvelope, TransactionId, args};
use tari_ootle_wallet_sdk::{
    models::TransactionStatus,
    network::{
        FetchedSubstates,
        SubstateQueryResult,
        TransactionFinalizedResult,
        TransactionFinalizedStream,
        TransactionQueryResult,
        UtxoUpdateStream,
        WalletNetworkInterface,
    },
    storage::{ReadableWalletStore, TagAndPublicNoncePair, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_wallet_sdk_services::{account_monitor::AccountScanner, notify::Notify};
use tari_template_abi::TemplateDef;
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::types::{
    ResourceAddress,
    SubstateOwnerRule,
    TemplateAddress,
    UtxoAddress,
    UtxoId,
    access_rules::ComponentAccessRules,
    constants::STEALTH_TARI_RESOURCE_ADDRESS,
};
use time::{OffsetDateTime, PrimitiveDateTime};

use crate::support::{CannedTransactionResultInterface, PanicError, Test, TestSdkSpec, TestWithNetwork};

fn now() -> PrimitiveDateTime {
    let now = OffsetDateTime::now_utc();
    PrimitiveDateTime::new(now.date(), now.time())
}

fn build_transaction() -> Transaction {
    Transaction::builder_localnet(Epoch(100))
        .allocate_component_address("component")
        .put_last_instruction_output_on_workspace("bucket")
        .call_method("component", "new", args!["bucket"])
        .build_and_seal(&RistrettoSecretKey::from(1))
}

fn vault_value() -> SubstateValue {
    Vault::new(ResourceContainer::public_fungible(
        STEALTH_TARI_RESOURCE_ADDRESS,
        100u64,
    ))
    .into()
}

fn account_component_value() -> SubstateValue {
    Component {
        header: ComponentHeader {
            template_address: ACCOUNT_TEMPLATE_ADDRESS,
            owner_rule: SubstateOwnerRule::None,
            access_rules: ComponentAccessRules::new(),
            entity_id: Default::default(),
        },
        body: ComponentBody::empty(),
    }
    .into()
}

fn utxo_id() -> SubstateId {
    UtxoAddress::new(STEALTH_TARI_RESOURCE_ADDRESS, UtxoId::from_array([1; 32])).into()
}

fn diff_with_up(id: SubstateId, value: SubstateValue) -> SubstateDiff {
    let mut diff = SubstateDiff::new();
    diff.up(id, Substate::new(1, value));
    diff
}

fn committed_result(transaction_id: TransactionId, diff: SubstateDiff) -> TransactionQueryResult {
    let finalize = FinalizeResult::new(
        transaction_id.into_array().into(),
        vec![],
        vec![],
        TransactionResult::Accept(diff),
        FeeReceipt::default(),
    );

    TransactionQueryResult {
        transaction_id,
        result: TransactionFinalizedResult::Finalized {
            final_decision: Decision::Commit,
            execution_result: Some(Box::new(ExecuteResult {
                finalize,
                execution_time: Duration::from_secs(1),
                execute_epoch: None,
                wasm_execution_points: 0,
                native_execution_points: 0,
            })),
            execution_time: Duration::from_secs(1),
            finalized_time: now(),
            abort_details: None,
        },
        receipt: None,
    }
}

#[tokio::test]
async fn a_finalized_diff_with_a_mistyped_substate_is_not_stored() {
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    let diff = diff_with_up(Test::test_account_address().into(), vault_value());

    let test = TestWithNetwork::with_network(CannedTransactionResultInterface::new(committed_result(
        transaction_id,
        diff,
    )));
    test.store()
        .with_write_tx(|tx| tx.transactions_insert(&transaction, None, &[Test::test_account_address()], false))
        .unwrap();

    test.sdk()
        .transaction_api()
        .check_and_store_finalized_transaction(transaction_id)
        .await
        .unwrap_err();

    let stored = test
        .store()
        .with_read_tx(|tx| tx.transactions_get(transaction_id))
        .unwrap();
    assert_eq!(stored.status, TransactionStatus::New);
    assert!(stored.finalize.is_none());
}

fn scanner<TNetwork: WalletNetworkInterface + Clone>(
    test: &TestWithNetwork<TNetwork>,
) -> AccountScanner<TestSdkSpec<TNetwork>> {
    AccountScanner::new(Notify::new(1), test.sdk().clone())
}

#[tokio::test]
async fn scanning_a_utxo_id_with_a_vault_value_is_an_error() {
    let test = Test::new();
    let diff = diff_with_up(utxo_id(), vault_value());

    scanner(&test)
        .process_result(TransactionId::default(), &diff, None)
        .await
        .unwrap_err();
}

#[tokio::test]
async fn scanning_a_vault_id_with_an_account_value_is_an_error() {
    let test = Test::new();
    let diff = diff_with_up(Test::test_vault_address().into(), account_component_value());

    scanner(&test)
        .process_result(TransactionId::default(), &diff, None)
        .await
        .unwrap_err();
}

#[tokio::test]
async fn refreshing_an_account_the_network_returns_as_a_vault_is_an_error() {
    let test = TestWithNetwork::with_network(FixedSubstateNetwork {
        substate: vault_value(),
    });

    scanner(&test)
        .refresh_account(Test::test_account_address())
        .await
        .unwrap_err();
}

/// Answers every substate query with `substate`, whatever id was asked for.
#[derive(Debug, Clone)]
struct FixedSubstateNetwork {
    substate: SubstateValue,
}

impl WalletNetworkInterface for FixedSubstateNetwork {
    type Error = PanicError;

    async fn query_substate(
        &self,
        _address: &SubstateId,
        _version: Option<SubstateVersion>,
        _local_search_only: bool,
        _include_proof: bool,
    ) -> Result<SubstateQueryResult, Self::Error> {
        Ok(SubstateQueryResult {
            version: SubstateVersion::new(1),
            substate: self.substate.clone(),
            proof: None,
        })
    }

    async fn get_substates(&self, _: Vec<SubstateId>, _: bool) -> Result<FetchedSubstates, Self::Error> {
        panic!("FixedSubstateNetwork called")
    }

    async fn submit_transaction(&self, _transaction: Transaction) -> Result<TransactionId, Self::Error> {
        panic!("FixedSubstateNetwork called")
    }

    async fn submit_transaction_envelope(
        &self,
        _transaction: TransactionEnvelope,
    ) -> Result<TransactionId, Self::Error> {
        panic!("FixedSubstateNetwork called")
    }

    async fn submit_dry_run_transaction(
        &self,
        _transaction: Transaction,
    ) -> Result<TransactionQueryResult, Self::Error> {
        panic!("FixedSubstateNetwork called")
    }

    async fn query_transaction_result(
        &self,
        _transaction_id: TransactionId,
        _include_proof: bool,
    ) -> Result<TransactionQueryResult, Self::Error> {
        panic!("FixedSubstateNetwork called")
    }

    async fn subscribe_transaction_finalized(&self) -> Result<TransactionFinalizedStream<Self::Error>, Self::Error> {
        Ok(futures::stream::pending().boxed())
    }

    async fn fetch_template_definition(&self, _template_address: TemplateAddress) -> Result<TemplateDef, Self::Error> {
        panic!("FixedSubstateNetwork called")
    }

    async fn stream_stealth_utxo_updates(
        &self,
        _from_epoch: tari_ootle_common_types::Epoch,
        _resource_address: ResourceAddress,
        _shard_state_versions: Vec<(Shard, StateVersion)>,
        _unspent_only: bool,
    ) -> Result<UtxoUpdateStream<Self::Error>, Self::Error> {
        panic!("FixedSubstateNetwork called")
    }

    async fn list_watched_substates(
        &self,
        _template_address: Option<TemplateAddress>,
        _limit: Option<u64>,
        _offset: Option<u64>,
    ) -> Result<Vec<WatchedSubstateItem>, Self::Error> {
        panic!("FixedSubstateNetwork called")
    }

    async fn get_unspent_utxos(
        &self,
        _resource_address: ResourceAddress,
        _tag_and_nonce_pairs: Vec<TagAndPublicNoncePair>,
    ) -> Result<Vec<(UtxoId, Utxo)>, Self::Error> {
        panic!("FixedSubstateNetwork called")
    }

    async fn get_latest_epoch_checkpoint(&self, _shard: Shard) -> Result<EpochCheckpointEntry, Self::Error> {
        panic!("FixedSubstateNetwork called")
    }

    async fn list_epoch_checkpoints(
        &self,
        _from_epoch: tari_ootle_common_types::Epoch,
        _limit: u32,
        _shard: Shard,
    ) -> Result<Vec<EpochCheckpointEntry>, Self::Error> {
        panic!("FixedSubstateNetwork called")
    }

    async fn get_committee(
        &self,
        _epoch: tari_ootle_common_types::Epoch,
        _shard: Shard,
    ) -> Result<ValidatorSetCommittee, Self::Error> {
        panic!("FixedSubstateNetwork called")
    }

    async fn get_current_epoch(&self) -> Result<tari_ootle_common_types::Epoch, Self::Error> {
        panic!("FixedSubstateNetwork called")
    }

    async fn wait_until_ready(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}
