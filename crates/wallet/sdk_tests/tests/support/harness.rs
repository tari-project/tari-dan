//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, convert::Infallible, future::Future, marker::PhantomData, str::FromStr};

use futures::StreamExt;
use ootle_byte_type::ToByteType;
use tari_crypto::tari_utilities::SafePassword;
use tari_engine_types::{
    Utxo,
    crypto::commit_amount,
    resource::Resource,
    substate::{Substate, SubstateId},
};
use tari_indexer_client::types::WatchedSubstateItem;
use tari_ootle_address::Network;
use tari_ootle_common_types::{
    Epoch,
    StateVersion,
    optional::{IsNotFoundError, Optional},
    response_status::{ResponseErrorStatus, TransactionStatusResponseError},
    shard::Shard,
};
use tari_ootle_transaction::{Transaction, TransactionEnvelope, TransactionId};
use tari_ootle_wallet_sdk::{
    WalletSdk,
    WalletSdkConfig,
    WalletSdkSpec,
    cipher_seed::CipherSeedRestore,
    local_key_store::LocalKeyStore,
    models::{
        ConfidentialOutputModel,
        EpochBirthday,
        KeyBranch,
        KeyId,
        OutputStatus,
        WalletLockDropGuard,
        WalletLockId,
    },
    network::{
        SubstateQueryResult,
        TransactionFinalizedStream,
        TransactionQueryResult,
        UtxoUpdateStream,
        WalletNetworkInterface,
    },
    storage::TagAndPublicNoncePair,
};
use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;
use tari_template_abi::TemplateDef;
use tari_template_lib::types::{
    Amount,
    ComponentAddress,
    EncryptedData,
    Metadata,
    ResourceAddress,
    ResourceType,
    SubstateOwnerRule,
    TemplateAddress,
    UtxoId,
    VaultId,
    access_rules::ResourceAccessRules,
    constants::{STEALTH_TARI_RESOURCE_ADDRESS, TOKEN_SYMBOL},
    crypto::PedersenCommitmentBytes,
};

pub struct TestSdkSpec<TNetwork = PanicNetworkInterface>(PhantomData<TNetwork>);

impl<TNetwork: WalletNetworkInterface> WalletSdkSpec for TestSdkSpec<TNetwork> {
    type KeyStore = LocalKeyStore;
    type NetworkInterface = TNetwork;
    type Store = SqliteWalletStore;
}

pub type Test = TestWithNetwork<PanicNetworkInterface>;

pub struct TestWithNetwork<TNetwork: WalletNetworkInterface> {
    store: SqliteWalletStore,
    sdk: WalletSdk<TestSdkSpec<TNetwork>>,
    _temp: tempfile::TempDir,
}

impl Test {
    pub fn new() -> Self {
        Self::with_network(PanicNetworkInterface)
    }
}

impl<TNetwork: WalletNetworkInterface> TestWithNetwork<TNetwork> {
    pub fn with_network(network: TNetwork) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let store = SqliteWalletStore::try_open(temp.path().join("data/wallet.sqlite")).unwrap();
        store.run_migrations().unwrap();

        let mut sdk = WalletSdk::initialize_with_local_key_store(
            store.clone(),
            network,
            WalletSdkConfig {
                network: Network::LocalNet,
                override_keyring_password: Some(SafePassword::from_str("SuuuCh Sekret W0W").unwrap()),
            },
            EpochBirthday::new(1200.try_into().unwrap(), u64::MAX),
        )
        .unwrap();
        sdk.initialize_cipher_seed(CipherSeedRestore::CreateNewIfRequired)
            .unwrap();
        let accounts_api = sdk.accounts_api();
        sdk.resources_api()
            .upsert_resource(
                &STEALTH_TARI_RESOURCE_ADDRESS,
                &Resource::new(
                    ResourceType::Stealth,
                    SubstateOwnerRule::None,
                    ResourceAccessRules::new(),
                    Metadata::from([(TOKEN_SYMBOL, "TEST".to_string())]),
                    None,
                    None,
                    6,
                    false,
                ),
            )
            .unwrap();
        accounts_api
            .add_account(
                Some("test"),
                &Test::test_account_address(),
                KeyId::derived(KeyBranch::ViewOnlyKey, 0),
                KeyId::derived(KeyBranch::Account, 0),
                Epoch::zero(),
                true,
                true,
            )
            .unwrap();
        accounts_api
            .add_vault(
                Test::test_account_address(),
                Test::test_vault_address(),
                0,
                STEALTH_TARI_RESOURCE_ADDRESS,
                ResourceType::Stealth,
                Some("TEST".to_string()),
                6,
            )
            .unwrap();

        Self {
            store,
            sdk,
            _temp: temp,
        }
    }

    pub fn test_account_address() -> ComponentAddress {
        "component_0dc41b5cc74b36d696c7b140323a40a2f98b71df5d60e5a6bf4c1a07ffffffff"
            .parse()
            .unwrap()
    }

    pub fn test_vault_address() -> VaultId {
        "vault_0dc41b5cc74b36d696c7b140323a40a2f98b71df5d60e5a6bf4c1a07ffffffff"
            .parse()
            .unwrap()
    }

    pub fn add_unspent_output<A: Into<Amount>>(&self, amount: A) -> PedersenCommitmentBytes {
        let amount = amount.into();

        let outputs_api = self.sdk.confidential_outputs_api();
        let commitment = commit_amount(&Default::default(), amount)
            .expect("Cannot add unspent output with negative amount")
            .to_byte_type();
        outputs_api
            .add_output(ConfidentialOutputModel {
                account_address: Self::test_account_address(),
                vault_id: Self::test_vault_address(),
                commitment,
                value: amount,
                sender_public_nonce: None,
                view_only_key_id: KeyId::derived(KeyBranch::ViewOnlyKey, 0),
                owner_key_id: Some(KeyId::derived(KeyBranch::Account, 0)),
                encrypted_data: EncryptedData::try_from(vec![0; EncryptedData::min_size()]).unwrap(),
                public_asset_tag: None,
                memo: None,
                status: OutputStatus::Unspent,
                lock_id: None,
            })
            .unwrap();
        commitment
    }

    pub fn new_lock(&self) -> WalletLockDropGuard<'_, SqliteWalletStore> {
        self.sdk.locks_api().create_lock().unwrap()
    }

    pub fn get_unspent_balance(&self) -> Amount {
        let outputs_api = self.sdk.confidential_outputs_api();
        outputs_api
            .get_unspent_balance(&Test::test_vault_address())
            .optional()
            .unwrap()
            .unwrap_or_default()
    }

    pub fn sdk(&self) -> &WalletSdk<TestSdkSpec<TNetwork>> {
        &self.sdk
    }

    pub fn store(&self) -> &SqliteWalletStore {
        &self.store
    }
}

/// Network interface that answers `query_transaction_result` with a canned result and panics on every other call.
#[derive(Debug, Clone)]
pub struct CannedTransactionResultInterface {
    result: TransactionQueryResult,
}

impl CannedTransactionResultInterface {
    pub fn new(result: TransactionQueryResult) -> Self {
        Self { result }
    }
}

impl WalletNetworkInterface for CannedTransactionResultInterface {
    type Error = PanicError;

    #[allow(clippy::diverging_sub_expression)]
    async fn query_substate(
        &self,
        _address: &SubstateId,
        _version: Option<u64>,
        _local_search_only: bool,
    ) -> Result<SubstateQueryResult, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    async fn get_substates(&self, _: Vec<SubstateId>) -> Result<HashMap<SubstateId, Substate>, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    #[allow(clippy::diverging_sub_expression)]
    async fn submit_transaction(&self, _transaction: Transaction) -> Result<TransactionId, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    async fn submit_transaction_envelope(
        &self,
        _transaction: TransactionEnvelope,
    ) -> Result<TransactionId, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    #[allow(clippy::diverging_sub_expression)]
    async fn submit_dry_run_transaction(
        &self,
        _transaction: Transaction,
    ) -> Result<TransactionQueryResult, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    async fn query_transaction_result(
        &self,
        _transaction_id: TransactionId,
    ) -> Result<TransactionQueryResult, Self::Error> {
        Ok(self.result.clone())
    }

    async fn subscribe_transaction_finalized(&self) -> Result<TransactionFinalizedStream<Self::Error>, Self::Error> {
        Ok(futures::stream::pending().boxed())
    }

    async fn fetch_template_definition(&self, _template_address: TemplateAddress) -> Result<TemplateDef, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    async fn stream_stealth_utxo_updates(
        &self,
        _from_epoch: Epoch,
        _resource_address: ResourceAddress,
        _shard_state_versions: Vec<(Shard, StateVersion)>,
        _unspent_only: bool,
    ) -> Result<UtxoUpdateStream<Self::Error>, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    async fn list_watched_substates(
        &self,
        _template_address: Option<TemplateAddress>,
        _limit: Option<u64>,
        _offset: Option<u64>,
    ) -> Result<Vec<WatchedSubstateItem>, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    async fn get_unspent_utxos(
        &self,
        _resource_address: ResourceAddress,
        _tag_and_nonce_pairs: Vec<TagAndPublicNoncePair>,
    ) -> Result<Vec<(UtxoId, Utxo)>, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    async fn get_current_epoch(&self) -> Result<Epoch, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    async fn wait_until_ready(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PanicNetworkInterface;

// TODO: test the substate scanning in the SDK
impl WalletNetworkInterface for PanicNetworkInterface {
    type Error = PanicError;

    #[allow(clippy::diverging_sub_expression)]
    async fn query_substate(
        &self,
        _address: &SubstateId,
        _version: Option<u64>,
        _local_search_only: bool,
    ) -> Result<SubstateQueryResult, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    async fn get_substates(&self, _: Vec<SubstateId>) -> Result<HashMap<SubstateId, Substate>, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    #[allow(clippy::diverging_sub_expression)]
    async fn submit_transaction(&self, _transaction: Transaction) -> Result<TransactionId, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    async fn submit_transaction_envelope(
        &self,
        _transaction: TransactionEnvelope,
    ) -> Result<TransactionId, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    #[allow(clippy::diverging_sub_expression)]
    async fn submit_dry_run_transaction(
        &self,
        _transaction: Transaction,
    ) -> Result<TransactionQueryResult, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    #[allow(clippy::diverging_sub_expression)]
    async fn query_transaction_result(
        &self,
        _transaction_id: TransactionId,
    ) -> Result<TransactionQueryResult, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    async fn subscribe_transaction_finalized(&self) -> Result<TransactionFinalizedStream<Self::Error>, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    async fn fetch_template_definition(&self, _template_address: TemplateAddress) -> Result<TemplateDef, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    async fn stream_stealth_utxo_updates(
        &self,
        _from_epoch: Epoch,
        _resource_address: ResourceAddress,
        _shard_state_versions: Vec<(Shard, StateVersion)>,
        _unspent_only: bool,
    ) -> Result<UtxoUpdateStream<Self::Error>, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    async fn list_watched_substates(
        &self,
        _template_address: Option<TemplateAddress>,
        _limit: Option<u64>,
        _offset: Option<u64>,
    ) -> Result<Vec<WatchedSubstateItem>, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    async fn get_unspent_utxos(
        &self,
        _resource_address: ResourceAddress,
        _tag_and_nonce_pairs: Vec<TagAndPublicNoncePair>,
    ) -> Result<Vec<(UtxoId, Utxo)>, Self::Error> {
        panic!("PanicNetworkInterface get_unspent_utxos called")
    }

    async fn get_current_epoch(&self) -> Result<Epoch, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    async fn wait_until_ready(&self) -> Result<(), Self::Error> {
        panic!("PanicNetworkInterface called")
    }
}

#[derive(Debug)]
pub enum PanicError {}

impl std::fmt::Display for PanicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PanicError")
    }
}

impl std::error::Error for PanicError {}

impl TransactionStatusResponseError for PanicError {
    fn get_status(&self) -> ResponseErrorStatus {
        panic!("get_status called on PanicError")
    }

    fn get_error_message(&self) -> String {
        panic!("get_error_message called on PanicError")
    }
}

impl IsNotFoundError for PanicError {
    fn is_not_found_error(&self) -> bool {
        false
    }
}
