//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{convert::Infallible, time::Duration};

use async_trait::async_trait;
use tari_common_types::types::Commitment;
use tari_crypto::commitment::HomomorphicCommitmentFactory;
use tari_dan_common_types::optional::Optional;
use tari_dan_wallet_sdk::{
    confidential::get_commitment_factory,
    models::{ConfidentialOutputModel, ConfidentialProofId, OutputStatus},
    network::{SubstateQueryResult, TransactionQueryResult, WalletNetworkInterface},
    storage::{WalletStore, WalletStoreReader},
    DanWalletSdk,
    WalletSdkConfig,
};
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;
use tari_engine_types::substate::SubstateId;
use tari_template_lib::{
    constants::CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
    models::{Amount, EncryptedData},
    resource::ResourceType,
};
use tari_transaction::{SubstateRequirement, Transaction, TransactionId};

#[test]
fn outputs_locked_and_released() {
    let test = Test::new();

    let commitment_25 = test.add_unspent_output(25);
    let commitment_49 = test.add_unspent_output(49);
    let _commitment_100 = test.add_unspent_output(100);

    let proof_id = test.new_proof();
    let (inputs, total_value) = test
        .sdk()
        .confidential_outputs_api()
        .lock_outputs_by_amount(&Test::test_vault_address(), Amount(50), proof_id, false)
        .unwrap();
    assert_eq!(total_value, 74);
    assert_eq!(inputs.len(), 2);

    let locked = test
        .store()
        .with_read_tx(|tx| tx.outputs_get_locked_by_proof(proof_id))
        .unwrap();

    assert!(locked.iter().any(|l| l.commitment == commitment_25));
    assert!(locked.iter().any(|l| l.commitment == commitment_49));
    assert_eq!(locked.len(), 2);

    test.sdk
        .confidential_outputs_api()
        .release_proof_outputs(proof_id)
        .unwrap();

    let locked = test
        .store()
        .with_read_tx(|tx| tx.outputs_get_locked_by_proof(proof_id))
        .unwrap();
    assert_eq!(locked.len(), 0);
}

#[test]
fn outputs_locked_and_finalized() {
    let test = Test::new();

    let commitment_25 = test.add_unspent_output(25);
    let commitment_49 = test.add_unspent_output(49);
    let commitment_100 = test.add_unspent_output(100);

    let outputs_api = test.sdk().confidential_outputs_api();
    let proof_id = test.new_proof();

    let (inputs, total_value) = outputs_api
        .lock_outputs_by_amount(&Test::test_vault_address(), Amount(50), proof_id, false)
        .unwrap();
    assert_eq!(total_value, 74);
    assert_eq!(inputs.len(), 2);

    let locked = test
        .store()
        .with_read_tx(|tx| tx.outputs_get_locked_by_proof(proof_id))
        .unwrap();

    assert!(locked.iter().any(|l| l.commitment == commitment_25));
    assert!(locked.iter().any(|l| l.commitment == commitment_49));
    assert_eq!(locked.len(), 2);

    // Add a change output belonging to this proof
    let commitment_change = get_commitment_factory().commit_value(&Default::default(), 24);
    outputs_api
        .add_output(ConfidentialOutputModel {
            account_address: Test::test_account_address(),
            vault_address: Test::test_vault_address(),
            commitment: commitment_change.clone(),
            value: 24,
            sender_public_nonce: None,
            encryption_secret_key_index: 0,
            encrypted_data: EncryptedData([0; EncryptedData::size()]),
            public_asset_tag: None,
            status: OutputStatus::LockedUnconfirmed,
            locked_by_proof: Some(proof_id),
        })
        .unwrap();

    let balance = test.get_unspent_balance();
    assert_eq!(balance, 100);

    outputs_api.finalize_outputs_for_proof(proof_id).unwrap();

    {
        let mut tx = test.store().create_read_tx().unwrap();
        let locked = tx.outputs_get_locked_by_proof(proof_id).unwrap();
        assert_eq!(locked.len(), 0);

        let unspent = tx
            .outputs_get_by_account_and_status(&Test::test_account_address(), OutputStatus::Unspent)
            .unwrap();
        assert!(unspent.iter().any(|l| l.commitment == commitment_change));
        assert!(unspent.iter().any(|l| l.commitment == commitment_100));
        assert_eq!(unspent.len(), 2);
        let balance = tx.outputs_get_unspent_balance(&Test::test_vault_address()).unwrap();
        assert_eq!(balance, 124);
    }
}

// -------------------------------- Test Harness -------------------------------- //

struct Test {
    store: SqliteWalletStore,
    sdk: DanWalletSdk<SqliteWalletStore, PanicIndexer>,
    _temp: tempfile::TempDir,
}

impl Test {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let store = SqliteWalletStore::try_open(temp.path().join("data/wallet.sqlite")).unwrap();
        store.run_migrations().unwrap();

        let sdk = DanWalletSdk::initialize(store.clone(), PanicIndexer, WalletSdkConfig {
            password: None,
            jwt_expiry: Duration::from_secs(60),
            jwt_secret_key: "secret_key".to_string(),
        })
        .unwrap();
        let accounts_api = sdk.accounts_api();
        accounts_api
            .add_account(Some("test"), &Test::test_account_address(), 0, true)
            .unwrap();
        accounts_api
            .add_vault(
                Test::test_account_address(),
                Test::test_vault_address(),
                CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
                ResourceType::Confidential,
                Some("TEST".to_string()),
            )
            .unwrap();

        Self {
            store,
            sdk,
            _temp: temp,
        }
    }

    pub fn test_account_address() -> SubstateId {
        "component_0dc41b5cc74b36d696c7b140323a40a2f98b71df5d60e5a6bf4c1a071d15f562"
            .parse()
            .unwrap()
    }

    pub fn test_vault_address() -> SubstateId {
        "vault_0dc41b5cc74b36d696c7b140323a40a2f98b71df5d60e5a6bf4c1a071d15f562"
            .parse()
            .unwrap()
    }

    pub fn add_unspent_output(&self, amount: u64) -> Commitment {
        let outputs_api = self.sdk.confidential_outputs_api();
        let commitment = get_commitment_factory().commit_value(&Default::default(), amount);
        outputs_api
            .add_output(ConfidentialOutputModel {
                account_address: Self::test_account_address(),
                vault_address: Self::test_vault_address(),
                commitment: commitment.clone(),
                value: amount,
                sender_public_nonce: None,
                encryption_secret_key_index: 0,
                encrypted_data: EncryptedData([0; EncryptedData::size()]),
                public_asset_tag: None,
                status: OutputStatus::Unspent,
                locked_by_proof: None,
            })
            .unwrap();
        commitment
    }

    pub fn new_proof(&self) -> ConfidentialProofId {
        let outputs_api = self.sdk.confidential_outputs_api();
        outputs_api.add_proof(&Self::test_vault_address()).unwrap()
    }

    pub fn get_unspent_balance(&self) -> u64 {
        let outputs_api = self.sdk.confidential_outputs_api();
        outputs_api
            .get_unspent_balance(&Test::test_vault_address())
            .optional()
            .unwrap()
            .unwrap_or(0)
    }

    pub fn sdk(&self) -> &DanWalletSdk<SqliteWalletStore, PanicIndexer> {
        &self.sdk
    }

    pub fn store(&self) -> &SqliteWalletStore {
        &self.store
    }
}

#[derive(Debug, Clone)]
struct PanicIndexer;

// TODO: test the substate scanning in the SDK
#[async_trait]
impl WalletNetworkInterface for PanicIndexer {
    type Error = Infallible;

    #[allow(clippy::diverging_sub_expression)]
    async fn query_substate(
        &self,
        _address: &SubstateId,
        _version: Option<u32>,
        _local_search_only: bool,
    ) -> Result<SubstateQueryResult, Self::Error> {
        panic!("PanicIndexer called")
    }

    #[allow(clippy::diverging_sub_expression)]
    async fn submit_transaction(
        &self,
        _transaction: Transaction,
        _required_substates: Vec<SubstateRequirement>,
    ) -> Result<TransactionId, Self::Error> {
        panic!("PanicIndexer called")
    }

    #[allow(clippy::diverging_sub_expression)]
    async fn submit_dry_run_transaction(
        &self,
        _transaction: Transaction,
        _required_substates: Vec<SubstateRequirement>,
    ) -> Result<TransactionQueryResult, Self::Error> {
        panic!("PanicIndexer called")
    }

    #[allow(clippy::diverging_sub_expression)]
    async fn query_transaction_result(
        &self,
        _transaction_id: TransactionId,
    ) -> Result<TransactionQueryResult, Self::Error> {
        panic!("PanicIndexer called")
    }
}
