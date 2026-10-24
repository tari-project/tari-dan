//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum_extra::headers::authorization::Bearer;
use dashmap::DashMap;
use tari_ootle_transaction::{Epoch, Transaction, TransactionBuilder, UnsignedTransaction};
use tari_ootle_wallet_sdk::{
    models::{LockOwner, SigningRequester, WalletEvent},
    network::WalletNetworkInterface,
};
use tari_ootle_wallet_sdk_services::{
    account_monitor::AccountMonitorHandle,
    notify::Notify,
    transaction_service::TransactionServiceHandle,
};
use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;
use tari_ootle_walletd_client::permissions::{Permission, Permissions};
use tari_shutdown::ShutdownSignal;
use tari_utilities::SafePassword;
use url::Url;
use webauthn_rs::Webauthn;

use crate::{
    WalletSdk,
    config::WalletDaemonConfig,
    handlers::{
        auth::{
            WalletAuthenticator,
            api_keys,
            cookie,
            jwt::{AuthError, JwtApi, enforce_scopes},
        },
        helpers::invalid_params,
    },
    services::{RefreshTokenStore, WebauthnService},
};

/// An authenticated caller: what it may do, (for an API key) what an Admin
/// named it, whether it holds a delegated token, or whether it presented the
/// local `.cookie` token. Neither the name, delegation, nor cookie origin
/// affects scope checks here.
#[derive(Debug, Clone)]
pub struct AuthIdentity {
    pub permissions: Permissions,
    pub api_key_name: Option<String>,
    pub api_key_id: Option<i32>,
    /// The caller presented a token `webrtc.start` minted for a connected app.
    pub delegated: bool,
    /// Which `webrtc.start` call minted the delegated token.
    pub delegation_id: Option<u64>,
    /// The caller presented the daemon's local `.cookie` token.
    pub local_cookie: bool,
}

/// Reserved `requested_by` sentinel recorded when a transaction request is
/// created by a local `.cookie` bearer. `handle_create_api_key` refuses to mint
/// an API key with this name so the two cannot collide.
pub const LOCAL_COOKIE_REQUESTER: &str = "local-cookie";

impl AuthIdentity {
    /// Who a lock handed back to this caller belongs to.
    pub fn lock_owner(&self) -> LockOwner {
        match self.api_key_id {
            Some(id) => LockOwner::ApiKey(id),
            None if self.delegated => LockOwner::Delegated(self.delegation_id.unwrap_or_default()),
            None if self.local_cookie => LockOwner::LocalCookie,
            None => LockOwner::Wallet,
        }
    }

    /// Typed requester identity for signing and transaction requests.
    pub fn signing_requester(&self) -> SigningRequester {
        match (&self.api_key_name, self.delegated, self.local_cookie) {
            (Some(name), _, _) => SigningRequester::ApiKey { name: name.clone() },
            (None, true, _) => SigningRequester::ConnectedApp,
            (None, false, true) => SigningRequester::LocalClient,
            (None, false, false) => SigningRequester::WalletSession,
        }
    }

    /// Label recorded on a transaction request (`requested_by`) when this caller creates one.
    pub fn requested_by(&self) -> Option<String> {
        match self.signing_requester() {
            SigningRequester::ApiKey { name } => Some(name),
            SigningRequester::LocalClient => Some(LOCAL_COOKIE_REQUESTER.to_string()),
            SigningRequester::ConnectedApp | SigningRequester::WalletSession => None,
        }
    }
}

#[derive(Debug, Clone)]
struct CachedEpoch {
    epoch: Epoch,
    read_at: Instant,
    indexer: Url,
}

#[derive(Debug, Clone)]
pub struct HandlerContext {
    wallet_sdk: WalletSdk,
    notifier: Notify<WalletEvent>,
    transaction_service: TransactionServiceHandle,
    account_monitor: AccountMonitorHandle,
    config: WalletDaemonConfig,
    jwt_secret: SafePassword,
    cookie: Option<(SafePassword, Permissions)>,
    authenticator: WalletAuthenticator,
    refresh_token_store: RefreshTokenStore,
    shutdown_signal: ShutdownSignal,
    /// In-memory map of `api_key.id` → most-recent `last_used_at` bump time.
    /// The auth shim runs on every authenticated request, so the hot path
    /// must avoid spawning a tokio task and grabbing the SQLite write lock
    /// when a recent bump has already happened. This table lets the common
    /// case short-circuit with a single sharded read; the DB-level
    /// throttle in `api_key_touch_last_used` stays as belt-and-braces
    /// against process restart and racing shim invocations.
    api_key_last_used_bumps: Arc<DashMap<i32, Instant>>,
    /// Last epoch read from the network, with the time it was read. Every transaction build needs
    /// the current epoch to stamp `max_epoch`; epochs turn over on the order of tens of minutes, so
    /// a short cache keeps a burst of builds from making an indexer round-trip each.
    cached_epoch: Arc<Mutex<Option<CachedEpoch>>>,
}

/// How long a read of the current epoch is reused before the network is asked again. Well under an
/// epoch, so the stamped window is never short by more than this.
const EPOCH_CACHE_TTL: Duration = Duration::from_secs(30);

impl HandlerContext {
    pub fn new(
        wallet_sdk: WalletSdk,
        notifier: Notify<WalletEvent>,
        transaction_service: TransactionServiceHandle,
        account_monitor: AccountMonitorHandle,
        config: WalletDaemonConfig,
        authenticator: WalletAuthenticator,
        jwt_secret: SafePassword,
        shutdown_signal: ShutdownSignal,
    ) -> Self {
        Self {
            wallet_sdk,
            notifier,
            transaction_service,
            account_monitor,
            config,
            authenticator,
            refresh_token_store: RefreshTokenStore::new(Duration::from_secs(60 * 60)),
            jwt_secret,
            cookie: None,
            shutdown_signal,
            api_key_last_used_bumps: Arc::new(DashMap::new()),
            cached_epoch: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_cookie_token(mut self, cookie: Option<(SafePassword, Permissions)>) -> Self {
        self.cookie = cookie;
        self
    }

    pub fn notifier(&self) -> &Notify<WalletEvent> {
        &self.notifier
    }

    pub fn wallet_sdk(&self) -> &WalletSdk {
        &self.wallet_sdk
    }

    fn check_cookie_auth(&self, raw_token: &str) -> Result<Permissions, AuthError> {
        let (expected, permissions) = self.cookie.as_ref().ok_or(AuthError::AccessDeniedInvalidBearerToken)?;
        if !cookie::verify_cookie_token(expected, raw_token) {
            return Err(AuthError::AccessDeniedInvalidBearerToken);
        }
        Ok(permissions.clone())
    }

    /// Resolve an incoming bearer token and return its granted permissions
    /// **without** checking any specific requirement. Use this when a
    /// handler needs to resolve a request entity before scope-checking
    /// (e.g. `accounts.get` looks up the account so it can require
    /// `accounts:read:<addr>`); call [`enforce_scopes`] or
    /// [`Permissions::check`] on the returned set with the resolved scope.
    /// For the common "check these permissions" case, prefer [`authorize`].
    ///
    /// Bearer tokens prefixed with the API key marker (`tw_…`) are
    /// resolved directly against the `api_keys` table — there is no
    /// `auth.request` round-trip or JWT exchange for agent credentials.
    /// Bearer tokens prefixed with the cookie marker (`tc_…`) are verified
    /// in constant time against the daemon's ephemeral cookie token.
    /// Every other bearer is treated as a JWT.
    ///
    /// The `tw_` prefix is a routing hint, not a security boundary. A
    /// malformed `tw_` string fails by hash-miss in exactly the same way a
    /// non-`tw_` string does (and surfaces the same `ApiKeyInvalidOrRevoked`
    /// error to avoid enumeration leaks).
    pub fn check_auth(&self, token: Option<&Bearer>) -> Result<Permissions, AuthError> {
        let bearer = token.ok_or(AuthError::AccessDeniedNoBearerToken)?;
        if bearer.token().starts_with(api_keys::API_KEY_PREFIX) {
            let row = api_keys::find_active_by_raw(self.wallet_sdk.store(), bearer.token())
                .map_err(AuthError::from)?
                .ok_or(AuthError::ApiKeyInvalidOrRevoked)?;
            let granted = api_keys::parse_permissions(&row.permissions).map_err(|e| {
                // A persisted row whose permissions column doesn't parse is a
                // data-integrity bug, not an authentication failure — log the
                // actual reason but surface the same opaque error so an
                // attacker can't distinguish.
                log::warn!(
                    target: "tari::ootle::walletd::auth",
                    "API key {} has unparseable permissions column: {e}",
                    row.id,
                );
                AuthError::ApiKeyInvalidOrRevoked
            })?;
            self.maybe_spawn_last_used_bump(row.id);
            return Ok(granted);
        }
        if bearer.token().starts_with(cookie::COOKIE_TOKEN_PREFIX) {
            return self.check_cookie_auth(bearer.token());
        }
        self.jwt_api().check_auth(Some(bearer))
    }

    /// Validate the bearer and enforce the required permissions in one
    /// call. Equivalent to `check_auth(token)?` then `enforce_scopes(...)`.
    /// Most handlers should use this; the split form is for handlers that
    /// need to resolve a request entity before scope-checking.
    pub fn authorize(&self, token: Option<&Bearer>, required: &[Permission]) -> Result<Permissions, AuthError> {
        let granted = self.check_auth(token)?;
        enforce_scopes(&granted, required)?;
        Ok(granted)
    }

    /// Like [`authorize`], but also reports *which* API key presented the
    /// credential, so a handler can record it for a human to read later.
    ///
    /// The name is the one an Admin chose when minting the key, so a tool
    /// cannot name itself: a caller-supplied label rendered next to a value
    /// transfer would be a phishing surface. It is display and audit only --
    /// nothing authorises on it, and a JWT session simply has no name.
    pub fn authorize_with_identity(
        &self,
        token: Option<&Bearer>,
        required: &[Permission],
    ) -> Result<AuthIdentity, AuthError> {
        let identity = self.check_auth_with_identity(token)?;
        enforce_scopes(&identity.permissions, required)?;
        Ok(identity)
    }

    /// Like [`check_auth`], but returns the caller's identity alongside its
    /// permissions. See [`authorize_with_identity`].
    pub fn check_auth_with_identity(&self, token: Option<&Bearer>) -> Result<AuthIdentity, AuthError> {
        let bearer = token.ok_or(AuthError::AccessDeniedNoBearerToken)?;
        let identity = if bearer.token().starts_with(api_keys::API_KEY_PREFIX) {
            let row = api_keys::find_active_by_raw(self.wallet_sdk.store(), bearer.token())
                .map_err(AuthError::from)?
                .ok_or(AuthError::ApiKeyInvalidOrRevoked)?;
            let granted = api_keys::parse_permissions(&row.permissions).map_err(|e| {
                log::warn!(
                    target: "tari::ootle::walletd::auth",
                    "API key {} has unparseable permissions column: {e}",
                    row.id,
                );
                AuthError::ApiKeyInvalidOrRevoked
            })?;
            self.maybe_spawn_last_used_bump(row.id);
            AuthIdentity {
                permissions: granted,
                api_key_name: Some(row.name),
                api_key_id: Some(row.id),
                delegated: false,
                delegation_id: None,
                local_cookie: false,
            }
        } else if bearer.token().starts_with(cookie::COOKIE_TOKEN_PREFIX) {
            let granted = self.check_cookie_auth(bearer.token())?;
            AuthIdentity {
                permissions: granted,
                api_key_name: None,
                api_key_id: None,
                delegated: false,
                delegation_id: None,
                local_cookie: true,
            }
        } else {
            let claims = self.jwt_api().check_auth_claims(Some(bearer))?;
            AuthIdentity {
                permissions: claims.permissions,
                api_key_name: None,
                api_key_id: None,
                delegated: claims.delegated,
                delegation_id: claims.delegation_id,
                local_cookie: false,
            }
        };

        Ok(identity)
    }

    /// Like [`check_auth`], but rejects API-key bearers, cookie tokens, and
    /// delegated tokens (those `webrtc.start` mints for a peer). Use for
    /// endpoints that must NOT be reachable with a programmatically minted or
    /// ambient local credential — key export, API-key management endpoints
    /// (create / list / revoke) and transaction/signing-request decisions.
    /// Restricting them to an interactive session means a leaked Admin API key
    /// or local `.cookie` cannot export keys, mint further keys to survive a
    /// revoke/restart, or approve a signature/transaction request.
    pub fn check_auth_user_only(&self, token: Option<&Bearer>) -> Result<Permissions, AuthError> {
        let bearer = token.ok_or(AuthError::AccessDeniedNoBearerToken)?;
        if bearer.token().starts_with(api_keys::API_KEY_PREFIX) ||
            bearer.token().starts_with(cookie::COOKIE_TOKEN_PREFIX)
        {
            return Err(AuthError::UserAuthOnly);
        }
        self.jwt_api().check_user_auth(Some(bearer))
    }

    /// Convenience: [`check_auth_user_only`] + [`enforce_scopes`].
    pub fn authorize_user_only(
        &self,
        token: Option<&Bearer>,
        required: &[Permission],
    ) -> Result<Permissions, AuthError> {
        let granted = self.check_auth_user_only(token)?;
        enforce_scopes(&granted, required)?;
        Ok(granted)
    }

    /// Spawn a `last_used_at` write only when the in-memory throttle says
    /// the previous bump for this id is outside the throttle window. Common
    /// case (busy agent, bump within window) is a single sharded DashMap
    /// read and an early return — no spawn, no DB lock, no executor wakeup.
    ///
    /// The DB-level filter on `api_key_touch_last_used` is kept as
    /// belt-and-braces for process restart, multi-daemon-against-same-DB,
    /// and the racy case where two concurrent shim calls both see a stale
    /// in-memory entry.
    fn maybe_spawn_last_used_bump(&self, id: i32) {
        let now = Instant::now();
        // `saturating_duration_since` avoids the panic that
        // `Instant::duration_since` would raise on a non-monotonic clock
        // (rare on Linux but observed under VM suspend/resume). A request
        // handler must not panic over a missed throttle observation.
        if let Some(prev) = self.api_key_last_used_bumps.get(&id) &&
            now.saturating_duration_since(*prev) < api_keys::LAST_USED_BUMP_THROTTLE
        {
            return;
        }
        // Update before spawning so a second call in the same window
        // short-circuits even if the spawned task hasn't run yet.
        self.api_key_last_used_bumps.insert(id, now);

        let store = self.wallet_sdk.store().clone();
        // `spawn_blocking`, not `spawn`: the closure runs synchronous diesel
        // I/O. The blocking-pool keeps the executor's worker threads free
        // for actual async work.
        tokio::task::spawn_blocking(move || {
            if let Err(e) = api_keys::touch_last_used_throttled(&store, id, api_keys::LAST_USED_BUMP_THROTTLE) {
                log::warn!(
                    target: "tari::ootle::walletd::auth",
                    "Failed to bump api_keys.last_used_at for id {id}: {e}",
                );
            }
        });
    }

    pub fn jwt_api(&self) -> JwtApi<'_> {
        JwtApi::new(self.config().jwt_expiry, &self.jwt_secret)
    }

    pub fn shutdown_signal(&self) -> &ShutdownSignal {
        &self.shutdown_signal
    }

    pub fn account_monitor(&self) -> &AccountMonitorHandle {
        &self.account_monitor
    }

    pub fn transaction_service(&self) -> &TransactionServiceHandle {
        &self.transaction_service
    }

    pub fn config(&self) -> &WalletDaemonConfig {
        &self.config
    }

    /// Refuses a `max_fee` above the wallet's configured `max_fee_limit`. Every handler that submits
    /// a transaction at a caller's `max_fee` calls this before building it.
    pub fn enforce_max_fee_limit(&self, max_fee: u64) -> Result<(), anyhow::Error> {
        let limit = self.config.max_fee_limit;
        if max_fee > limit {
            return Err(invalid_params(
                "max_fee",
                Some(format!(
                    "{max_fee} µT is above this wallet's max_fee_limit of {limit} µT. Lower max_fee, or raise \
                     max_fee_limit in the wallet daemon config"
                )),
            ));
        }
        Ok(())
    }

    pub fn authenticator(&self) -> &WalletAuthenticator {
        &self.authenticator
    }

    pub fn refresh_token_store(&self) -> &RefreshTokenStore {
        &self.refresh_token_store
    }

    pub fn webauthn_service(&self) -> Option<&WebauthnService<SqliteWalletStore>> {
        self.authenticator.webauthn_service()
    }

    pub fn webauthn(&self) -> Option<&Webauthn> {
        self.authenticator.webauthn()
    }

    /// The current epoch, re-read from the network at most every [`EPOCH_CACHE_TTL`].
    ///
    /// The cached value belongs to the indexer it was read from and is discarded once another indexer is in use,
    /// whether by `settings.set` or by failover: stamping a `max_epoch` derived from one indexer's view onto a
    /// transaction submitted through another can yield a window that the network will not accept.
    pub async fn current_epoch(&self) -> Result<Epoch, anyhow::Error> {
        let network = self.wallet_sdk.get_network_interface();
        let indexer = network.get_endpoint();
        if let Some(cached) = &*self.cached_epoch.lock().unwrap() &&
            cached.indexer == indexer &&
            cached.read_at.elapsed() < EPOCH_CACHE_TTL
        {
            return Ok(cached.epoch);
        }

        let epoch = network.get_current_epoch().await?;
        *self.cached_epoch.lock().unwrap() = Some(CachedEpoch {
            epoch,
            read_at: Instant::now(),
            indexer,
        });
        Ok(epoch)
    }

    /// The `max_epoch` this wallet stamps on transactions it builds: the current epoch plus the
    /// configured validity window.
    pub async fn transaction_max_epoch(&self) -> Result<Epoch, anyhow::Error> {
        let current_epoch = self.current_epoch().await?;
        Ok(Epoch(
            current_epoch
                .as_u64()
                .saturating_add(self.config().default_transaction_validity_epochs),
        ))
    }

    /// A builder seeded from a caller-supplied transaction.
    ///
    /// The caller has already chosen everything this would otherwise resolve — including its own
    /// `max_epoch` — so unlike [`Self::transaction_builder`] this needs no epoch and makes no
    /// network call. Submitting a pre-built transaction therefore does not depend on the indexer
    /// being reachable.
    pub fn transaction_builder_from_unsigned<T: Into<UnsignedTransaction>>(
        &self,
        transaction: T,
    ) -> TransactionBuilder {
        TransactionBuilder::from_unsigned(transaction)
    }

    /// Returns a TransactionBuilder with the current network configured.
    ///
    /// The builder is stamped with a random nonce so that repeated identical intents (same
    /// instructions, inputs and signer) produce distinct transaction ids. Paths that submit
    /// caller-provided bytes replace the whole unsigned transaction via
    /// `with_unsigned_transaction`, which discards the stamp — the caller's bytes are preserved
    /// verbatim there.
    ///
    /// `max_epoch` is stamped `default_transaction_validity_epochs` ahead of the current epoch.
    /// Building therefore depends on the network being reachable: without a current epoch there is
    /// no way to choose a window the network will accept, so this fails rather than guessing.
    /// Callers that want a different window override it with `with_max_epoch`.
    pub async fn transaction_builder(&self) -> Result<TransactionBuilder, anyhow::Error> {
        let max_epoch = self.transaction_max_epoch().await?;
        Ok(Transaction::builder(self.config().network.as_byte(), max_epoch).with_nonce(rand::random()))
    }
}

#[cfg(test)]
mod max_fee_limit_tests {
    use std::str::FromStr;

    use axum_extra::headers::Authorization;
    use tari_ootle_address::Network;
    use tari_ootle_common_types::Epoch;
    use tari_ootle_wallet_sdk::{
        IndexerTrust,
        WalletSdkConfig,
        cipher_seed::CipherSeedRestore,
        models::{EpochBirthday, KeyBranch, KeyId},
    };
    use tari_ootle_wallet_sdk_services::{
        account_monitor::AccountMonitor,
        indexer_rest_api::IndexerRestApiNetworkInterface,
        transaction_service::TransactionService,
        utxo_scanner::StealthUtxoScannerWorker,
    };
    use tari_ootle_walletd_client::{
        ComponentAddressOrName,
        types::{CallInstructionRequest, ClaimBurnProof, ClaimBurnRequest, PublishTemplateRequest},
    };
    use tari_shutdown::Shutdown;
    use tari_template_lib_types::ComponentAddress;

    use super::*;
    use crate::{
        config::WalletDaemonAuth,
        handlers::{accounts::handle_claim_burn, auth::create_authenticator, transaction::*},
    };

    const LIMIT: u64 = 10_000;

    /// A handler context with one account and `max_fee_limit = LIMIT`. The indexer URL points at a
    /// closed port, so a request that gets past the limit fails fast on its first network call.
    struct LimitTest {
        context: HandlerContext,
        account: ComponentAddress,
        bearer: Bearer,
        _temp: tempfile::TempDir,
    }

    async fn setup() -> LimitTest {
        let temp = tempfile::tempdir().unwrap();
        let store = SqliteWalletStore::try_open(temp.path().join("wallet.sqlite")).unwrap();
        store.run_migrations().unwrap();
        let mut sdk = WalletSdk::initialize_with_local_key_store(
            store.clone(),
            IndexerRestApiNetworkInterface::new("http://127.0.0.1:1"),
            WalletSdkConfig {
                network: Network::LocalNet,
                override_keyring_password: Some(SafePassword::from_str("test wallet password").unwrap()),
                indexer_trust: IndexerTrust::default(),
            },
            EpochBirthday::far_future(),
        )
        .unwrap();
        sdk.initialize_cipher_seed(CipherSeedRestore::CreateNewIfRequired)
            .unwrap();
        let account = ComponentAddress::from_array([0xaa; 32]);
        sdk.accounts_api()
            .add_account(
                Some("payer"),
                &account,
                KeyId::derived(KeyBranch::ViewOnlyKey, 0),
                KeyId::derived(KeyBranch::Account, 0),
                Epoch::zero(),
                false,
                true,
            )
            .unwrap();

        let notify = Notify::new(10);
        let shutdown = Shutdown::new();
        let (transaction_service, transaction_service_handle) =
            TransactionService::new(notify.clone(), sdk.clone(), shutdown.to_signal());
        let (utxo_worker, utxo_scanner_handle) = StealthUtxoScannerWorker::new(sdk.clone(), notify.clone()).spawn();
        let (account_monitor, account_monitor_handle) =
            AccountMonitor::new(notify.clone(), sdk.clone(), utxo_scanner_handle, shutdown.to_signal());
        let mut config = WalletDaemonConfig::default();
        config.network = Network::LocalNet;
        config.authentication = WalletDaemonAuth::None;
        config.max_fee_limit = LIMIT;
        let context = HandlerContext::new(
            sdk,
            notify,
            transaction_service_handle,
            account_monitor_handle,
            config.clone(),
            create_authenticator(&config, store).unwrap(),
            SafePassword::from_str("test jwt secret").unwrap(),
            shutdown.to_signal(),
        );

        // These handlers need only the context, so the background workers shut down now.
        shutdown.trigger();
        drop(account_monitor);
        drop(transaction_service);
        utxo_worker.abort();
        drop(utxo_worker.await);

        let claims = context
            .jwt_api()
            .generate_auth_claims(Permissions::from_str("admin").unwrap())
            .unwrap();
        let bearer = Authorization::<Bearer>::bearer(&context.jwt_api().grant(&claims).unwrap())
            .unwrap()
            .0;
        LimitTest {
            context,
            account,
            bearer,
            _temp: temp,
        }
    }

    fn is_over_the_limit(err: &anyhow::Error) -> bool {
        err.to_string().contains("max_fee_limit")
    }

    fn call_instruction(test: &LimitTest, max_fee: u64) -> CallInstructionRequest {
        CallInstructionRequest {
            instructions: vec![],
            fee_account: ComponentAddressOrName::ComponentAddress(test.account),
            max_fee,
            inputs: vec![],
            override_inputs: None,
            new_outputs: None,
            proof_ids: vec![],
            min_epoch: None,
            max_epoch: None,
        }
    }

    fn publish(test: &LimitTest, max_fee: u64, dry_run: bool) -> PublishTemplateRequest {
        PublishTemplateRequest {
            binary: vec![0],
            fee_account: Some(ComponentAddressOrName::ComponentAddress(test.account)),
            max_fee,
            detect_inputs: false,
            dry_run,
            metadata: None,
        }
    }

    fn claim_burn(test: &LimitTest, max_fee: u64, is_dry_run: bool) -> ClaimBurnRequest {
        ClaimBurnRequest {
            account: ComponentAddressOrName::ComponentAddress(test.account),
            claim_proof: ClaimBurnProof::FromFile {
                file_name: "absent.json".to_string(),
            },
            max_fee,
            is_dry_run,
        }
    }

    #[tokio::test]
    async fn a_submission_over_the_limit_is_refused_before_it_is_built() {
        let test = setup().await;
        let bearer = Some(&test.bearer);

        let err = handle_submit_instruction(&test.context, bearer, call_instruction(&test, LIMIT + 1))
            .await
            .unwrap_err();
        assert!(is_over_the_limit(&err), "submit_instruction: {err}");

        let err = handle_publish_template(&test.context, bearer, publish(&test, LIMIT + 1, false))
            .await
            .unwrap_err();
        assert!(is_over_the_limit(&err), "publish_template: {err}");

        let err = handle_claim_burn(&test.context, bearer, claim_burn(&test, LIMIT + 1, false))
            .await
            .unwrap_err();
        assert!(is_over_the_limit(&err), "claim_burn: {err}");
    }

    #[tokio::test]
    async fn a_submission_at_the_limit_gets_past_it() {
        let test = setup().await;
        let err = handle_submit_instruction(&test.context, Some(&test.bearer), call_instruction(&test, LIMIT))
            .await
            .expect_err("the offline indexer fails the request after the limit");
        assert!(!is_over_the_limit(&err), "{err}");
    }

    /// A dry run commits nothing, so callers may cap it at any figure, e.g. the account's balance
    /// while learning what a publish costs.
    #[tokio::test]
    async fn a_dry_run_is_exempt() {
        let test = setup().await;
        let bearer = Some(&test.bearer);

        let err = handle_publish_template(&test.context, bearer, publish(&test, u64::MAX, true))
            .await
            .expect_err("the offline indexer fails the request after the limit");
        assert!(!is_over_the_limit(&err), "publish_template: {err}");

        let err = handle_claim_burn(&test.context, bearer, claim_burn(&test, u64::MAX, true))
            .await
            .expect_err("the absent proof file fails the request after the limit");
        assert!(!is_over_the_limit(&err), "claim_burn: {err}");
    }
}

#[cfg(test)]
mod cookie_auth_tests {
    use axum_extra::headers::Authorization;
    use tari_ootle_walletd_client::{
        ComponentAddressOrName,
        permissions::Crud,
        types::{
            AccountsExportViewKeyRequest,
            AuthCreateApiKeyRequest,
            AuthListApiKeysRequest,
            AuthRevokeApiKeyRequest,
            SigningRequestDecisionRequest,
            TransactionRequestDecisionRequest,
        },
    };

    use super::*;
    use crate::handlers::{
        accounts::handle_export_view_key,
        auth::{handle_create_api_key, handle_list_api_keys, handle_revoke_api_key},
        signing_requests::handle_approve as handle_approve_signing_request,
        test_support::TestDaemon,
        transaction_requests::handle_approve as handle_approve_transaction_request,
    };

    #[tokio::test]
    async fn cookie_bearer_authenticates_and_rejects_wrong_cookie() {
        let daemon = TestDaemon::start().await;

        let granted = daemon
            .context
            .check_auth(Some(&daemon.cookie))
            .expect("valid cookie must authenticate");
        assert!(granted.has_permission(&Permission::Admin));

        let identity = daemon
            .context
            .check_auth_with_identity(Some(&daemon.cookie))
            .expect("valid cookie must resolve identity");
        assert_eq!(identity.lock_owner(), LockOwner::LocalCookie);
        assert_eq!(identity.signing_requester(), SigningRequester::LocalClient);
        assert_eq!(identity.requested_by().as_deref(), Some(LOCAL_COOKIE_REQUESTER));
        assert!(identity.api_key_name.is_none());
        assert!(!identity.delegated);
        assert!(identity.local_cookie);

        // Minting an API key with the reserved `local-cookie` sentinel name is refused.
        let err = handle_create_api_key(&daemon.context, Some(&daemon.session), AuthCreateApiKeyRequest {
            name: LOCAL_COOKIE_REQUESTER.to_string(),
            permissions: vec!["accounts:read".to_string()],
            expires_at: None,
            confirm_admin: false,
        })
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("reserved"),
            "expected reserved-name error, got: {err}"
        );

        let wrong_cookie = Authorization::<Bearer>::bearer("tc_wrong_token_value_0123456789abcdef012345")
            .unwrap()
            .0;
        assert!(matches!(
            daemon.context.check_auth(Some(&wrong_cookie)),
            Err(AuthError::AccessDeniedInvalidBearerToken)
        ));
    }

    #[tokio::test]
    async fn cookie_bearer_is_rejected_by_user_only_endpoints() {
        let daemon = TestDaemon::start().await;
        let cookie = Some(&daemon.cookie);

        assert!(matches!(
            daemon.context.check_auth_user_only(cookie),
            Err(AuthError::UserAuthOnly)
        ));

        let err = handle_export_view_key(&daemon.context, cookie, AccountsExportViewKeyRequest {
            account: ComponentAddressOrName::Name("default".to_string()),
        })
        .await
        .unwrap_err();
        assert!(matches!(err.downcast_ref::<AuthError>(), Some(AuthError::UserAuthOnly)));

        let err = handle_create_api_key(&daemon.context, cookie, AuthCreateApiKeyRequest {
            name: "from-cookie".to_string(),
            permissions: vec!["accounts:read".to_string()],
            expires_at: None,
            confirm_admin: false,
        })
        .await
        .unwrap_err();
        assert!(matches!(err.downcast_ref::<AuthError>(), Some(AuthError::UserAuthOnly)));

        let err = handle_list_api_keys(&daemon.context, cookie, AuthListApiKeysRequest {
            include_revoked: false,
        })
        .await
        .unwrap_err();
        assert!(matches!(err.downcast_ref::<AuthError>(), Some(AuthError::UserAuthOnly)));

        let err = handle_revoke_api_key(&daemon.context, cookie, AuthRevokeApiKeyRequest { id: 1 })
            .await
            .unwrap_err();
        assert!(matches!(err.downcast_ref::<AuthError>(), Some(AuthError::UserAuthOnly)));

        let err = handle_approve_transaction_request(&daemon.context, cookie, TransactionRequestDecisionRequest {
            request_id: 1,
        })
        .await
        .unwrap_err();
        assert!(matches!(err.downcast_ref::<AuthError>(), Some(AuthError::UserAuthOnly)));

        let err =
            handle_approve_signing_request(&daemon.context, cookie, SigningRequestDecisionRequest { request_id: 1 })
                .await
                .unwrap_err();
        assert!(matches!(err.downcast_ref::<AuthError>(), Some(AuthError::UserAuthOnly)));
    }

    #[tokio::test]
    async fn cookie_scopes_and_cookie_auth_flag_are_enforced() {
        use std::str::FromStr;

        let daemon = TestDaemon::start().await;
        let scoped_context = daemon.context.clone().with_cookie_token(Some((
            SafePassword::from_str(daemon.cookie.token()).unwrap(),
            Permissions::from_str("accounts:read").unwrap(),
        )));

        scoped_context
            .authorize(Some(&daemon.cookie), &[Permission::Accounts(Crud::Read, None)])
            .expect("in-scope cookie permission must pass");

        let err = scoped_context
            .authorize(Some(&daemon.cookie), &[Permission::Transactions(Crud::Create)])
            .expect_err("out-of-scope cookie permission must be rejected");
        assert!(matches!(err, AuthError::InsufficientPermissions { .. }));

        let disabled_context = daemon.context.clone().with_cookie_token(None);
        assert!(matches!(
            disabled_context.check_auth(Some(&daemon.cookie)),
            Err(AuthError::AccessDeniedInvalidBearerToken)
        ));
    }
}
