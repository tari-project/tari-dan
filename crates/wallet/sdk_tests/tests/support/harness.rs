//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, convert::Infallible, future::Future, marker::PhantomData, str::FromStr};

use futures::StreamExt;
use ootle_byte_type::ToByteType;
use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey, tari_utilities::SafePassword};
use tari_engine_types::{
    Utxo,
    UtxoOutput,
    crypto::{OutputBody, commit_amount},
    resource::Resource,
    substate::{Substate, SubstateId},
};
use tari_indexer_client::types::{EpochCheckpointEntry, ValidatorSetCommittee, WatchedSubstateItem};
use tari_ootle_address::Network;
use tari_ootle_common_types::{
    Epoch,
    StateVersion,
    SubstateVersion,
    optional::{IsNotFoundError, Optional},
    response_status::{ResponseErrorStatus, TransactionStatusResponseError},
    shard::Shard,
};
use tari_ootle_transaction::{Transaction, TransactionEnvelope, TransactionId};
use tari_ootle_wallet_sdk::{
    IndexerTrust,
    WalletSdk,
    WalletSdkConfig,
    WalletSdkSpec,
    cipher_seed::CipherSeedRestore,
    local_key_store::LocalKeyStore,
    models::{
        AccountWithAddress,
        ConfidentialOutputModel,
        EpochBirthday,
        KeyBranch,
        KeyId,
        OutputStatus,
        WalletLockDropGuard,
        WalletLockId,
    },
    network::{
        FetchedSubstates,
        FetchedUtxos,
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
    stealth::SpendAuthorization,
};

use super::{random_key, random_keypair};

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
        Self::with_network_trusting(network, IndexerTrust::Trust)
    }

    /// A wallet with the test account, which trusts its indexer as far as `indexer_trust` says.
    pub fn with_network_trusting(network: TNetwork, indexer_trust: IndexerTrust) -> Self {
        let test = Self::without_accounts_trusting(network, indexer_trust);
        let accounts_api = test.sdk.accounts_api();
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
                SubstateVersion::ZERO,
                STEALTH_TARI_RESOURCE_ADDRESS,
                ResourceType::Stealth,
                Some("TEST".to_string()),
                6,
            )
            .unwrap();
        test
    }

    /// A wallet with the stealth TARI resource and no accounts, which takes its indexer at its word.
    pub fn without_accounts(network: TNetwork) -> Self {
        Self::without_accounts_trusting(network, IndexerTrust::Trust)
    }

    /// A wallet with the stealth TARI resource and no accounts, which trusts its indexer as far as `indexer_trust`
    /// says.
    pub fn without_accounts_trusting(network: TNetwork, indexer_trust: IndexerTrust) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let store = SqliteWalletStore::try_open(temp.path().join("data/wallet.sqlite")).unwrap();
        store.run_migrations().unwrap();

        let mut sdk = WalletSdk::initialize_with_local_key_store(
            store.clone(),
            network,
            WalletSdkConfig {
                network: Network::LocalNet,
                override_keyring_password: Some(SafePassword::from_str("SuuuCh Sekret W0W").unwrap()),
                indexer_trust,
            },
            EpochBirthday::new(1200.try_into().unwrap(), u64::MAX),
        )
        .unwrap();
        sdk.initialize_cipher_seed(CipherSeedRestore::CreateNewIfRequired)
            .unwrap();
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

    /// A stealth TARI UTXO of `amount` that `account` can read and spend.
    pub fn owned_utxo(&self, account: &AccountWithAddress, amount: u64) -> (UtxoId, Utxo) {
        let sdk = &self.sdk;
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
        let mask = random_key();
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

/// Network interface that answers `query_transaction_result` with a canned result, `get_current_epoch` with a canned
/// epoch and, if set to, `submit_transaction` with a rejection, and panics on every other call.
#[derive(Debug, Clone)]
pub struct CannedTransactionResultInterface {
    /// `None` answers that the transaction is not found.
    result: Option<TransactionQueryResult>,
    current_epoch: Option<Epoch>,
    rejects_submissions: bool,
}

impl CannedTransactionResultInterface {
    pub fn new(result: TransactionQueryResult) -> Self {
        Self {
            result: Some(result),
            current_epoch: None,
            rejects_submissions: false,
        }
    }

    /// An indexer that has no result for any transaction.
    pub fn not_found() -> Self {
        Self {
            result: None,
            current_epoch: None,
            rejects_submissions: false,
        }
    }

    pub fn with_current_epoch(mut self, epoch: Epoch) -> Self {
        self.current_epoch = Some(epoch);
        self
    }

    pub fn rejecting_submissions(mut self) -> Self {
        self.rejects_submissions = true;
        self
    }
}

impl WalletNetworkInterface for CannedTransactionResultInterface {
    type Error = PanicError;

    #[allow(clippy::diverging_sub_expression)]
    async fn query_substate(
        &self,
        _address: &SubstateId,
        _version: Option<SubstateVersion>,
        _local_search_only: bool,
        _include_proof: bool,
    ) -> Result<SubstateQueryResult, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    async fn get_substates(&self, _: Vec<SubstateId>, _: bool) -> Result<FetchedSubstates, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    async fn submit_transaction(&self, _transaction: Transaction) -> Result<TransactionId, Self::Error> {
        assert!(self.rejects_submissions, "CannedTransactionResultInterface called");
        Err(PanicError::Rejected)
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
        _include_proof: bool,
    ) -> Result<TransactionQueryResult, Self::Error> {
        self.result.clone().ok_or(PanicError::NotFound)
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

    async fn fetch_utxos(
        &self,
        _resource_address: ResourceAddress,
        _tag_and_nonce_pairs: Vec<TagAndPublicNoncePair>,
    ) -> Result<FetchedUtxos, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    async fn get_latest_epoch_checkpoint(&self, _shard: Shard) -> Result<EpochCheckpointEntry, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    async fn get_latest_shard_group_checkpoints(&self) -> Result<Vec<EpochCheckpointEntry>, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    async fn list_epoch_checkpoints(
        &self,
        _from_epoch: Epoch,
        _limit: u32,
        _shard: Shard,
    ) -> Result<Vec<EpochCheckpointEntry>, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    async fn get_committee(&self, _epoch: Epoch, _shard: Shard) -> Result<ValidatorSetCommittee, Self::Error> {
        panic!("CannedTransactionResultInterface called")
    }

    async fn get_current_epoch(&self) -> Result<Epoch, Self::Error> {
        Ok(self
            .current_epoch
            .expect("CannedTransactionResultInterface has no current epoch"))
    }

    async fn get_consensus_epoch(&self) -> Result<Option<Epoch>, Self::Error> {
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
        _version: Option<SubstateVersion>,
        _local_search_only: bool,
        _include_proof: bool,
    ) -> Result<SubstateQueryResult, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    async fn get_substates(&self, _: Vec<SubstateId>, _: bool) -> Result<FetchedSubstates, Self::Error> {
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
        _include_proof: bool,
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

    async fn fetch_utxos(
        &self,
        _resource_address: ResourceAddress,
        _tag_and_nonce_pairs: Vec<TagAndPublicNoncePair>,
    ) -> Result<FetchedUtxos, Self::Error> {
        panic!("PanicNetworkInterface fetch_utxos called")
    }

    async fn get_latest_epoch_checkpoint(&self, _shard: Shard) -> Result<EpochCheckpointEntry, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    async fn get_latest_shard_group_checkpoints(&self) -> Result<Vec<EpochCheckpointEntry>, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    async fn list_epoch_checkpoints(
        &self,
        _from_epoch: Epoch,
        _limit: u32,
        _shard: Shard,
    ) -> Result<Vec<EpochCheckpointEntry>, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    async fn get_committee(&self, _epoch: Epoch, _shard: Shard) -> Result<ValidatorSetCommittee, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    async fn get_current_epoch(&self) -> Result<Epoch, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    async fn get_consensus_epoch(&self) -> Result<Option<Epoch>, Self::Error> {
        panic!("PanicNetworkInterface called")
    }

    async fn wait_until_ready(&self) -> Result<(), Self::Error> {
        panic!("PanicNetworkInterface called")
    }
}

#[derive(Debug)]
pub enum PanicError {
    /// The errors a test network answers with in place of panicking.
    NotFound,
    Rejected,
}

impl std::fmt::Display for PanicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PanicError")
    }
}

impl std::error::Error for PanicError {}

impl TransactionStatusResponseError for PanicError {
    fn get_status(&self) -> ResponseErrorStatus {
        let message = self.get_error_message();
        match self {
            Self::NotFound => ResponseErrorStatus::NotFound { message },
            Self::Rejected => ResponseErrorStatus::TransactionRejected { message },
        }
    }

    fn get_error_message(&self) -> String {
        format!("{self:?}")
    }
}

impl IsNotFoundError for PanicError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::NotFound)
    }
}
