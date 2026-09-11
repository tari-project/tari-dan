//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, time::Duration};

use tari_engine_types::{
    resource::Resource,
    substate::{SubstateDiff, SubstateId},
};
use tari_ootle_common_types::{Epoch, StateVersion, VersionedSubstateIdRef, shard::Shard};
use tari_ootle_transaction::{Transaction, TransactionId, TransactionSignature, UnsignedTransaction};
use tari_template_lib::types::{
    Amount,
    ComponentAddress,
    NonFungibleId,
    ResourceAddress,
    TemplateAddress,
    UtxoAddress,
    UtxoId,
    VaultId,
    crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes, UtxoTag},
};
use webauthn_rs::prelude::Passkey;

use crate::{
    models::{
        AccountUpdate,
        AddressBookEntry,
        ApiKey,
        AuthoredTemplateModel,
        BalanceChangeSnapshot,
        BalanceChangeSource,
        ConfidentialOutputModel,
        ImportedKeyId,
        KeyId,
        KeyType,
        NewAccountData,
        NonFungibleToken,
        OutputStatus,
        StealthOutputModel,
        SubstateModel,
        TransactionRequestId,
        TransactionRequestModel,
        TransactionRequestStatus,
        UtxoUnspent,
        VaultModel,
        WalletEvent,
        WalletLockId,
        WalletTransactionUpdate,
    },
    storage::{CommittableStore, WalletStorageError},
};

pub trait WalletStoreWriter: CommittableStore {
    // Key manager
    fn key_manager_insert_or_ignore(&mut self, branch: &str, index: u64) -> Result<(), WalletStorageError>;
    fn key_manager_set_active_index(&mut self, branch: &str, index: u64) -> Result<(), WalletStorageError>;
    fn key_manager_reset_index(&mut self, branch: &str, index: u64) -> Result<(), WalletStorageError>;
    fn key_manager_insert_imported_key(
        &mut self,
        label: &str,
        public_key: &str,
        encrypted_key: &[u8],
        key_type: KeyType,
    ) -> Result<ImportedKeyId, WalletStorageError>;

    // Config
    fn config_set<T: serde::Serialize + ?Sized>(
        &mut self,
        key: &str,
        value: &T,
        is_encrypted: bool,
    ) -> Result<(), WalletStorageError>;

    // Transactions
    fn transactions_insert(
        &mut self,
        transaction: &Transaction,
        new_account_info: Option<&NewAccountData>,
        linked_accounts: &[ComponentAddress],
        is_dry_run: bool,
    ) -> Result<(), WalletStorageError>;
    fn transactions_update(&mut self, update: WalletTransactionUpdate<'_>) -> Result<(), WalletStorageError>;

    // Substates
    fn substates_upsert_root(
        &mut self,
        substate_id: VersionedSubstateIdRef<'_>,
        referenced_substates: HashSet<SubstateId>,
        module_name: Option<String>,
        template_addr: Option<TemplateAddress>,
    ) -> Result<(), WalletStorageError>;
    fn substates_upsert_child(
        &mut self,
        parent: &SubstateId,
        address: VersionedSubstateIdRef<'_>,
        referenced_substates: HashSet<SubstateId>,
    ) -> Result<(), WalletStorageError>;
    fn substates_remove(&mut self, substate: &SubstateId) -> Result<SubstateModel, WalletStorageError>;

    // Accounts
    fn accounts_set_default(&mut self, account_addr: &ComponentAddress) -> Result<(), WalletStorageError>;
    fn accounts_insert(
        &mut self,
        account_name: Option<&str>,
        account_addr: &ComponentAddress,
        view_only_key_id: KeyId,
        owner_key_id: Option<KeyId>,
        owner_public_key: &RistrettoPublicKeyBytes,
        associated_stealth_resources: &HashSet<ResourceAddress>,
        birthday_epoch: Epoch,
        is_confirmed_on_chain: bool,
        is_default: bool,
    ) -> Result<(), WalletStorageError>;

    fn accounts_update(
        &mut self,
        account_addr: &ComponentAddress,
        update: AccountUpdate<'_>,
    ) -> Result<(), WalletStorageError>;

    fn accounts_add_stealth_resource(
        &mut self,
        account_addr: &ComponentAddress,
        resource_address: ResourceAddress,
    ) -> Result<(), WalletStorageError>;

    // Vaults
    fn vaults_insert(&mut self, vault: VaultModel) -> Result<(), WalletStorageError>;
    fn vaults_update(
        &mut self,
        vault_id: VaultId,
        vault_version: u64,
        revealed_balance: Amount,
        confidential_balance: Amount,
    ) -> Result<(), WalletStorageError>;
    fn balance_changes_insert(
        &mut self,
        change: BalanceChangeSnapshot,
        source: BalanceChangeSource,
    ) -> Result<bool, WalletStorageError>;
    fn balance_changes_attribute_transaction(
        &mut self,
        vault_id: &VaultId,
        vault_version: u64,
        transaction_id: TransactionId,
    ) -> Result<bool, WalletStorageError>;
    fn vaults_lock_revealed_funds(
        &mut self,
        lock_id: WalletLockId,
        vault_id: &VaultId,
        amount_to_lock: Amount,
    ) -> Result<(), WalletStorageError>;
    fn vaults_finalized_locked_revealed_funds(&mut self, lock_id: WalletLockId) -> Result<(), WalletStorageError>;
    fn vaults_release_lock_revealed_funds(&mut self, lock_id: WalletLockId) -> Result<(), WalletStorageError>;
    // Resources
    fn resources_upsert(&mut self, address: &ResourceAddress, resource: &Resource) -> Result<(), WalletStorageError>;
    // Confidential Outputs
    fn confidential_outputs_lock_smallest_amount(
        &mut self,
        vault_id: &VaultId,
        lock_id: WalletLockId,
    ) -> Result<ConfidentialOutputModel, WalletStorageError>;
    fn confidential_outputs_insert(&mut self, output: ConfidentialOutputModel) -> Result<(), WalletStorageError>;
    /// Mark outputs as finalized
    fn confidential_outputs_finalize_by_lock_id(&mut self, lock_id: WalletLockId) -> Result<(), WalletStorageError>;
    /// Release outputs that were locked and remove pending unconfirmed outputs for this proof
    fn confidential_outputs_release_by_lock_id(&mut self, lock_id: WalletLockId) -> Result<(), WalletStorageError>;

    // Stealth Outputs
    fn stealth_outputs_lock_smallest_amount(
        &mut self,
        account_addr: &ComponentAddress,
        resource_address: &ResourceAddress,
        lock_id: WalletLockId,
    ) -> Result<StealthOutputModel, WalletStorageError>;

    fn stealth_outputs_lock_many(
        &mut self,
        resource_address: &ResourceAddress,
        utxos: &[&PedersenCommitmentBytes],
        lock_id: WalletLockId,
    ) -> Result<(), WalletStorageError>;

    /// Account-scoped lock: flips the named outputs to `LockedForSpend` and binds them to `lock_id`, restricted to
    /// rows owned by `account_address`. The account condition on the update is what makes caller-supplied commitments
    /// safe to lock: a commitment belonging to another account cannot be locked through this path even if it shares
    /// the resource. Errors (leaving the write transaction to roll back) unless every requested commitment matched an
    /// owned row.
    ///
    /// This method performs only the account-scoped status/lock flip; it does **not** independently validate
    /// spend-eligibility (e.g. `Unspent`/same-lock `LockedUnconfirmed` status, `owner_key_id`, `is_burnt`,
    /// `is_frozen`, `is_condition_spendable`). Callers must validate status and eligibility within the same write
    /// transaction before invoking this, otherwise ineligible outputs would be silently locked.
    fn stealth_outputs_lock_many_for_account(
        &mut self,
        account_address: &ComponentAddress,
        resource_address: &ResourceAddress,
        utxos: &[&PedersenCommitmentBytes],
        lock_id: WalletLockId,
    ) -> Result<(), WalletStorageError>;
    fn stealth_outputs_insert(&mut self, output: &StealthOutputModel) -> Result<(), WalletStorageError>;
    fn stealth_outputs_mark_as_spent(
        &mut self,
        resource_address: &ResourceAddress,
        id: &UtxoId,
    ) -> Result<(), WalletStorageError>;
    fn stealth_outputs_update(
        &mut self,
        address: &UtxoAddress,
        is_burnt: Option<bool>,
        status: Option<OutputStatus>,
        is_frozen: Option<bool>,
    ) -> Result<(), WalletStorageError>;

    // Transaction requests
    /// Persist a frozen request. The stored transaction is immutable: the
    /// approver views it and submit seals exactly it. The request is born
    /// [`TransactionRequestStatus::Pending`] and expires `ttl` from now.
    #[allow(clippy::too_many_arguments)]
    fn transaction_request_insert(
        &mut self,
        unsigned_transaction: &UnsignedTransaction,
        seal_signer: KeyId,
        other_signers: &[KeyId],
        signatures: &[TransactionSignature],
        lock_ids: &[WalletLockId],
        requested_by: Option<&str>,
        ttl: Duration,
    ) -> Result<TransactionRequestModel, WalletStorageError>;

    /// Move a request from any of `from` to `to`, returning the updated
    /// request.
    ///
    /// The `from` check and the write are a single conditional UPDATE, so
    /// concurrent callers resolve to one winner rather than both believing
    /// they acted — this is both the approve guard and the `Submitting` claim
    /// that serializes concurrent submitters. A request not in `from` is left
    /// untouched and [`WalletStorageError::UnexpectedState`] is returned.
    fn transaction_request_transition(
        &mut self,
        id: TransactionRequestId,
        from: &[TransactionRequestStatus],
        to: TransactionRequestStatus,
    ) -> Result<TransactionRequestModel, WalletStorageError>;

    /// Move a claimed (`Submitting`) request to `Submitted`, recording the
    /// transaction it became. Guarded on `Submitting` in the same statement as
    /// the write.
    ///
    /// Separate from [`Self::transaction_request_transition`] because
    /// `Submitted` is the only state that carries data: a submitted request
    /// without its transaction id is a dead end, since nothing then links the
    /// approval to the transaction it authorised.
    fn transaction_request_mark_submitted(
        &mut self,
        id: TransactionRequestId,
        transaction_id: TransactionId,
    ) -> Result<TransactionRequestModel, WalletStorageError>;

    // Locks
    fn locks_create(&mut self, timeout: Option<Duration>) -> Result<WalletLockId, WalletStorageError>;

    fn locks_delete(&mut self, lock_id: WalletLockId) -> Result<(), WalletStorageError>;

    /// Set the lock's deadline to `timeout` from now, or `None` to make it
    /// exempt from [`Self::locks_release_stale`] entirely. Used to hold inputs
    /// across an approval window that outlives the deadline the
    /// transfer-selection handler chose.
    fn locks_set_timeout(&mut self, lock_id: WalletLockId, timeout: Option<Duration>)
    -> Result<(), WalletStorageError>;

    fn locks_link_transaction(
        &mut self,
        lock_id: WalletLockId,
        transaction_id: TransactionId,
    ) -> Result<(), WalletStorageError>;

    fn locks_release_stale(&mut self) -> Result<usize, WalletStorageError>;

    /// Release the lock including all outputs and vaults that were locked. Release is used when a transaction is
    /// aborted.
    fn locks_release(&mut self, lock_id: WalletLockId) -> Result<(), WalletStorageError>;
    /// Finalize the lock according to the provided diff. Any outputs and vaults locked by this lock and included in the
    /// diff are finalised (marked as unspent/funds removed/added as necessary). Any objects not included in the diff
    /// are reverted and released from the lock. This is used when a transaction is committed.
    fn locks_unlock_finalized(&mut self, lock_id: WalletLockId, diff: &SubstateDiff) -> Result<(), WalletStorageError>;

    // Non fungible tokens
    fn non_fungible_token_upsert(&mut self, non_fungible_token: &NonFungibleToken) -> Result<(), WalletStorageError>;
    fn non_fungible_token_remove(
        &mut self,
        vault_id: &VaultId,
        non_fungible_id: &NonFungibleId,
    ) -> Result<(), WalletStorageError>;

    // Webauthn registrations
    fn webauthn_reg_insert(&mut self, username: String, passkey: Passkey) -> Result<(), WalletStorageError>;

    // Authored templates
    fn authored_templates_insert(&mut self, model: AuthoredTemplateModel) -> Result<(), WalletStorageError>;
    fn shard_state_version_set_many<I: IntoIterator<Item = (Shard, StateVersion)>>(
        &mut self,
        account: &ComponentAddress,
        resource_address: &ResourceAddress,
        shard_state_versions: I,
    ) -> Result<(), WalletStorageError>;

    fn utxo_process_queue_extend<I: IntoIterator<Item = (ComponentAddress, UtxoUnspent)>>(
        &mut self,
        resource_address: &ResourceAddress,
        items: I,
    ) -> Result<(), WalletStorageError>;
    fn utxo_process_queue_remove_item(
        &mut self,
        resource_address: ResourceAddress,
        tag: UtxoTag,
        public_nonce: RistrettoPublicKeyBytes,
    ) -> Result<(), WalletStorageError>;

    // Address book
    fn address_book_insert(
        &mut self,
        name: &str,
        address: &str,
        note: Option<&str>,
    ) -> Result<AddressBookEntry, WalletStorageError>;
    fn address_book_update(
        &mut self,
        name: &str,
        new_name: Option<&str>,
        address: Option<&str>,
        note: Option<&str>,
    ) -> Result<AddressBookEntry, WalletStorageError>;
    fn address_book_delete(&mut self, name: &str) -> Result<(), WalletStorageError>;

    // API keys
    /// Persist a new API key. `key_hash` is the SHA-256 hex digest of the
    /// raw key bytes — the raw key itself is never passed to the storage
    /// layer. `permissions` is the textual `Permissions` form
    /// (comma-separated; the same format the JWT layer already uses).
    /// `expires_at` is `None` for a never-expiring key; otherwise the
    /// `find_active_by_hash` filter excludes the row once that timestamp
    /// has passed.
    fn api_key_insert(
        &mut self,
        name: &str,
        key_hash: &str,
        permissions: &str,
        expires_at: Option<time::PrimitiveDateTime>,
    ) -> Result<ApiKey, WalletStorageError>;
    /// Bump `last_used_at` on a key after a successful authentication, only if
    /// the stored timestamp is at least `throttle` old (or NULL). Best-effort:
    /// callers should not let a write error abort the request — the auth
    /// already succeeded, this just refreshes the UI hint. Pass
    /// `Duration::ZERO` to bump unconditionally.
    fn api_key_touch_last_used(&mut self, id: i32, throttle: std::time::Duration) -> Result<(), WalletStorageError>;
    /// Soft-delete a key by stamping `revoked_at`. The row is preserved so
    /// the admin UI can still show the historical `last_used_at` for
    /// already-revoked credentials.
    fn api_key_revoke(&mut self, id: i32) -> Result<(), WalletStorageError>;
}

pub trait WalletEventStoreWriter {
    fn append_wallet_event(&mut self, event: &WalletEvent) -> Result<(), WalletStorageError>;
}
