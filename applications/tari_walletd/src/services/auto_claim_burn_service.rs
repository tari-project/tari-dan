//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, path::PathBuf, time::Instant};

use anyhow::Context;
use log::*;
use notify::{
    Config,
    EventKind,
    RecommendedWatcher,
    RecursiveMode,
    Watcher,
    event::{AccessKind, AccessMode},
};
use tari_ootle_common_types::{Epoch, optional::Optional};
use tari_ootle_wallet_sdk::network::WalletNetworkInterface;
use tari_ootle_wallet_sdk_services::transaction_service::TransactionServiceHandle;
use tari_shutdown::ShutdownSignal;
use tari_sidechain::CompleteClaimBurnProof;
use tokio::{
    sync::mpsc,
    time,
    time::{Duration, MissedTickBehavior},
};

use crate::{
    WalletSdk,
    handlers::{accounts::execute_claim_burn, helpers::complete_burn_proof_to_contents},
};

const LOG_TARGET: &str = "tari::ootle::wallet_daemon::auto_claim_burn";
const EPOCH_CHECK_INTERVAL: Duration = Duration::from_secs(30);
/// Validity window for an unattended claim: a few epochs is ample for the submit itself, and a
/// claim that does not land in that time is retried with a fresh window.
const CLAIM_TRANSACTION_VALIDITY_EPOCHS: u64 = 3;
/// Maximum retries for network/submission errors (indexer unreachable, tx service down).
const MAX_RETRIES_NETWORK: u32 = 10;
/// Maximum retries for file read/parse errors (file still being written on macOS).
const MAX_RETRIES_FILE_READ: u32 = 1;
/// Maximum times a claim is deferred (dry run reports the burn "not yet claimable") before giving
/// up. Proofs carry the L1 mined-in epoch, so a claim is only attempted once the network is past
/// that epoch and normally succeeds; a deferral is a rare edge (older proof with no epoch, or an
/// epoch-boundary race), so a modest bound suffices. On give-up the file remains for a manual claim.
const MAX_RETRIES_DEFERRED: u32 = 20;
/// Marker phrase in the claim-burn verifier's rejection when the burn's L1 block has not yet been
/// synced by validators into a claimable epoch. Such a claim is valid but must wait, so matching
/// this phrase lets the service defer rather than drop it. See `TariClaimBurnProofVerifier`.
const BURN_NOT_YET_CLAIMABLE_MARKER: &str = "not yet claimable";

/// True if a dry-run rejection indicates the burn is valid but its L1 block is not yet synced into a
/// claimable epoch, as opposed to a genuinely invalid claim.
fn is_burn_not_yet_claimable(reject_reason: &str) -> bool {
    reject_reason.contains(BURN_NOT_YET_CLAIMABLE_MARKER)
}

/// The epoch a claim must be strictly past before it is claimable, given the proof's
/// `mined_in_epoch`. A proof without the field (older L1 wallet) yields `Epoch(0)` so it is
/// attempted immediately and the dry-run backstop defers it until claimable (bounded).
fn claim_after_epoch(mined_in_epoch: Option<u64>) -> Epoch {
    mined_in_epoch.map(Epoch).unwrap_or(Epoch(0))
}

/// Watches the burn proof directory for new JSON files and automatically submits claim burn
/// transactions on behalf of the wallet, deferring each claim until the burn's L1 block is
/// claimable on L2.
///
/// ## Epoch-safety
/// A claim is only valid once L2 validators have synced the L1 block containing the burn into a
/// committed epoch. That sync lags the L1 tip by `base_layer_confirmations` (e.g. 780 blocks on
/// mainnet), so submitting too early is rejected. Each proof file records the L1 epoch the burn was
/// mined in (`mined_in_epoch`); this service reads it and only submits once the network reports a
/// strictly later epoch. Proofs without that field (older L1 wallets) are attempted eagerly and
/// held back by the dry-run backstop until claimable, or dropped after `MAX_RETRIES_DEFERRED`
/// deferrals (the file stays for a manual claim via the wallet API).
///
/// ## Crash safety
/// Pending state is held in memory only. On restart the service re-scans the directory and
/// re-queues any unclaimed files; each has its claim epoch re-derived from the proof file, so a
/// restart never submits a burn before it is claimable. If a duplicate submission occurs — because
/// the previous claim was submitted but not yet finalized before the restart — the second
/// transaction will fail with "already claimed" on-chain. The existing [`ClaimBurnMonitor`] handles
/// that gracefully.
///
/// [`ClaimBurnMonitor`]: super::claim_burn_monitor::ClaimBurnMonitor
pub struct AutoClaimBurnService {
    sdk: WalletSdk,
    transaction_service: TransactionServiceHandle,
    burn_proof_dir: PathBuf,
    /// Maps file name → pending claim state: the L1 epoch the burn must be past before claiming
    /// (resolved from the proof file) plus retry/deferral counters.
    pending_claims: HashMap<String, PendingClaim>,
    shutdown_signal: ShutdownSignal,
}

impl AutoClaimBurnService {
    pub fn new(
        sdk: WalletSdk,
        transaction_service: TransactionServiceHandle,
        burn_proof_dir: PathBuf,
        shutdown_signal: ShutdownSignal,
    ) -> Self {
        Self {
            sdk,
            transaction_service,
            burn_proof_dir,
            pending_claims: HashMap::new(),
            shutdown_signal,
        }
    }

    pub async fn run(mut self) -> Result<(), anyhow::Error> {
        info!(
            target: LOG_TARGET,
            "🔥 Auto claim burn service started, watching {}",
            self.burn_proof_dir.display()
        );

        if let Err(e) = tokio::fs::create_dir_all(&self.burn_proof_dir).await {
            warn!(target: LOG_TARGET, "Failed to create burn proof directory: {}", e);
        }

        // Start the watcher BEFORE scanning so files that arrive during the scan are not missed.
        let (event_tx, mut event_rx) = mpsc::channel::<notify::Result<notify::Event>>(100);
        let mut watcher = RecommendedWatcher::new(
            move |res| {
                // The watcher callback runs on the watcher's own thread, so blocking_send is correct here.
                let _unused = event_tx.blocking_send(res);
            },
            Config::default(),
        )?;
        watcher.watch(&self.burn_proof_dir, RecursiveMode::NonRecursive)?;

        // Recover any files that were present before this service started (e.g. placed while the
        // daemon was offline). Each is re-queued unresolved; its claim epoch is read from the proof
        // on the next check, so a restart never submits a burn before it is claimable.
        self.scan_proof_dir().await;

        // First tick after EPOCH_CHECK_INTERVAL
        let mut epoch_check_interval =
            time::interval_at((Instant::now() + EPOCH_CHECK_INTERVAL).into(), EPOCH_CHECK_INTERVAL);
        epoch_check_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = self.shutdown_signal.wait() => {
                    info!(target: LOG_TARGET, "🔥 Auto claim burn service shutting down");
                    break Ok(());
                }
                Some(res) = event_rx.recv() => {
                    self.on_fs_event(res).await;
                }
                _ = epoch_check_interval.tick() => {
                    self.check_and_submit_pending().await;
                }
            }
        }
        // watcher is dropped here, which stops the OS-level file watch.
    }

    /// Scans `burn_proof_dir` for any `.json` files present at startup and queues them unresolved;
    /// each file's claim epoch is read from the proof itself on the next epoch check.
    async fn scan_proof_dir(&mut self) {
        let mut read_dir = match tokio::fs::read_dir(&self.burn_proof_dir).await {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                warn!(target: LOG_TARGET, "Failed to read burn proof directory: {}", e);
                return;
            },
        };

        loop {
            match read_dir.next_entry().await {
                Ok(Some(entry)) => {
                    let is_file = entry.file_type().await.map(|ft| ft.is_file()).unwrap_or(false);
                    if !is_file {
                        continue;
                    }
                    let path = entry.path();
                    if path.extension().is_some_and(|ext| ext == "json") &&
                        let Some(file_name) = path.file_name().and_then(|n| n.to_str())
                    {
                        info!(target: LOG_TARGET, "Found existing unclaimed burn proof: {}", file_name);
                        self.pending_claims.insert(file_name.to_string(), PendingClaim::new());
                    }
                },
                Ok(None) => break,
                Err(e) => {
                    warn!(target: LOG_TARGET, "Failed to read directory entry: {}", e);
                },
            }
        }

        if !self.pending_claims.is_empty() {
            info!(
                target: LOG_TARGET,
                "Startup scan queued {} unclaimed burn proof(s)",
                self.pending_claims.len()
            );
        }
    }

    async fn on_fs_event(&mut self, res: notify::Result<notify::Event>) {
        let event = match res {
            Ok(e) => e,
            Err(e) => {
                warn!(target: LOG_TARGET, "File watch error: {}", e);
                return;
            },
        };

        // React to:
        //  • Access(Close(Write))  — file fully written and closed (most reliable on Linux)
        //  • Create(_)             — file created (covers macOS FSEvents and Windows)
        //  • Modify(Name(To))      — atomic rename-into-directory (common for temp-file writes)
        let is_relevant = matches!(
            event.kind,
            EventKind::Access(AccessKind::Close(AccessMode::Write)) |
                EventKind::Create(_) |
                EventKind::Modify(notify::event::ModifyKind::Name(notify::event::RenameMode::To))
        );
        if !is_relevant {
            return;
        }

        for path in &event.paths {
            // Ignore anything outside the root of the burn_proof_dir (e.g. claimed/ or pending/)
            if path.parent() != Some(self.burn_proof_dir.as_path()) {
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if self.pending_claims.contains_key(file_name) {
                // Already queued (e.g. duplicate event for the same file).
                continue;
            }

            // Queue unresolved; the claim epoch is read from the proof file on the next interval
            // check (the file may still be mid-write at this point).
            info!(target: LOG_TARGET, "New burn proof detected: '{}'", file_name);
            self.pending_claims.insert(file_name.to_string(), PendingClaim::new());
        }
    }

    /// Resolves the claim epoch for any unresolved queued files by reading each proof file's
    /// `mined_in_epoch`: the burn is claimable once L2 is strictly past that epoch. Proofs without
    /// the field (older L1 wallets) become eligible immediately and rely on the dry-run backstop to
    /// defer until claimable. Files that are not yet readable are left unresolved and retried next
    /// interval.
    async fn resolve_claim_epochs(&mut self) {
        let unresolved: Vec<String> = self
            .pending_claims
            .iter()
            .filter(|&(_, pending)| pending.claim_after_epoch.is_none())
            .map(|(name, _)| name.clone())
            .collect();
        for file_name in unresolved {
            let after = match self.read_proof_file(&file_name).await {
                Ok(proof) => {
                    let after = claim_after_epoch(proof.mined_in_epoch);
                    match proof.mined_in_epoch {
                        Some(mined) => info!(
                            target: LOG_TARGET,
                            "Burn proof '{}' was mined in L1 epoch {}; will claim once L2 passes it (epoch {}).",
                            file_name,
                            mined,
                            mined.saturating_add(1),
                        ),
                        None => info!(
                            target: LOG_TARGET,
                            "Burn proof '{}' carries no mined-in epoch (older L1 wallet); attempting now and \
                             deferring if not yet claimable.",
                            file_name,
                        ),
                    }
                    after
                },
                Err(e) => {
                    let pending = self.pending_claims.get_mut(&file_name).expect("just iterated");
                    pending.retries += 1;
                    let retries = pending.retries;
                    if retries >= MAX_RETRIES_FILE_READ {
                        error!(
                            target: LOG_TARGET,
                            "Failed to read burn proof '{}' to resolve its claim epoch after {} attempts: {}. \
                             Removing from queue; the file remains in the burn proof directory for a manual claim.",
                            file_name,
                            retries,
                            e,
                        );
                        self.pending_claims.remove(&file_name);
                    } else {
                        debug!(
                            target: LOG_TARGET,
                            "Could not read burn proof '{}' (attempt {}/{}): {}; will retry next interval.",
                            file_name,
                            retries,
                            MAX_RETRIES_FILE_READ,
                            e,
                        );
                    }
                    continue;
                },
            };
            if let Some(pending) = self.pending_claims.get_mut(&file_name) {
                pending.claim_after_epoch = Some(after);
            }
        }
    }

    /// Resolves epochs for newly queued files, then attempts to submit each claim whose target
    /// epoch is strictly less than the current epoch.
    #[expect(clippy::too_many_lines)]
    async fn check_and_submit_pending(&mut self) {
        if self.pending_claims.is_empty() {
            return;
        }

        let current_epoch = match self.query_current_epoch().await {
            Ok(e) => e,
            Err(e) => {
                warn!(
                    target: LOG_TARGET,
                    "Failed to query current epoch, skipping claim check: {}",
                    e
                );
                return;
            },
        };

        self.resolve_claim_epochs().await;

        let ready: Vec<String> = self
            .pending_claims
            .iter()
            .filter(|&(_, pending)| pending.claim_after_epoch.is_some_and(|e| current_epoch > e))
            .map(|(name, _)| name.clone())
            .collect();

        for file_name in ready {
            match self.try_submit_claim(&file_name, current_epoch).await {
                Ok(tx_id) => {
                    info!(
                        target: LOG_TARGET,
                        "✅ Auto-submitted claim burn for '{}' (tx_id: {})",
                        file_name,
                        tx_id,
                    );
                    self.pending_claims.remove(&file_name);
                },
                Err(ClaimError::Permanent(e)) => {
                    error!(
                        target: LOG_TARGET,
                        "Permanent error auto-claiming '{}', removing from queue: {}. \
                         The file remains in the burn proof directory for manual retry.",
                        file_name,
                        e
                    );
                    self.pending_claims.remove(&file_name);
                },
                Err(ClaimError::Transient { error: e, max_retries }) => {
                    let pending = self.pending_claims.get_mut(&file_name).expect("just iterated");
                    pending.retries += 1;
                    if pending.retries >= max_retries {
                        error!(
                            target: LOG_TARGET,
                            "Giving up on '{}' after {} retries: {}. \
                             The file remains in the burn proof directory for manual retry.",
                            file_name,
                            pending.retries,
                            e,
                        );
                        self.pending_claims.remove(&file_name);
                    } else {
                        warn!(
                            target: LOG_TARGET,
                            "Transient error auto-claiming '{}' (attempt {}/{}), will retry next interval: {}",
                            file_name,
                            pending.retries,
                            max_retries,
                            e,
                        );
                    }
                },
                Err(ClaimError::Deferred) => {
                    let pending = self.pending_claims.get_mut(&file_name).expect("just iterated");
                    pending.deferrals += 1;
                    let deferrals = pending.deferrals;
                    if deferrals >= MAX_RETRIES_DEFERRED {
                        warn!(
                            target: LOG_TARGET,
                            "Giving up auto-claiming '{}' after {} deferrals: the burn is still not claimable. \
                             The file remains in the burn proof directory; submit the claim manually via the wallet API.",
                            file_name,
                            deferrals,
                        );
                        self.pending_claims.remove(&file_name);
                    } else if deferrals == 1 {
                        info!(
                            target: LOG_TARGET,
                            "⏳ Burn claim '{}' is not yet claimable; its L1 burn block has not been synced by \
                             validators yet. Will retry each interval (up to {} times).",
                            file_name,
                            MAX_RETRIES_DEFERRED,
                        );
                    } else {
                        debug!(
                            target: LOG_TARGET,
                            "Burn claim '{}' still not claimable (attempt {}/{}), will retry next interval.",
                            file_name,
                            deferrals,
                            MAX_RETRIES_DEFERRED,
                        );
                    }
                },
            }
        }
    }

    /// Reads and deserializes a burn proof file from `burn_proof_dir`.
    async fn read_proof_file(&self, file_name: &str) -> anyhow::Result<CompleteClaimBurnProof> {
        let path = self.burn_proof_dir.join(file_name);
        let file = tokio::fs::File::open(&path)
            .await
            .with_context(|| format!("Failed to open burn proof file: {}", path.display()))?;
        serde_json::from_reader(file.into_std().await)
            .with_context(|| format!("Failed to parse burn proof file: {}", path.display()))
    }

    async fn try_submit_claim(
        &self,
        file_name: &str,
        current_epoch: Epoch,
    ) -> Result<tari_ootle_transaction::TransactionId, ClaimError> {
        let max_epoch = claim_max_epoch(current_epoch);
        let complete_proof = self
            .read_proof_file(file_name)
            .await
            .map_err(|e| ClaimError::transient(e, MAX_RETRIES_FILE_READ))?;

        let proof_contents = complete_burn_proof_to_contents(complete_proof).map_err(ClaimError::Permanent)?;

        // Find the target account using the burn_public_key embedded in the proof.
        // This is the account whose owner key was used when burning on L1.
        let accounts_api = self.sdk.accounts_api();
        let account = accounts_api
            .get_account_by_public_key(&proof_contents.claim_proof.burn_public_key)
            .optional()
            .map_err(|e| ClaimError::transient(e.into(), MAX_RETRIES_NETWORK))?
            .ok_or_else(|| {
                ClaimError::Permanent(anyhow::anyhow!(
                    "No account found for burn_public_key '{}'. The burn was not destined for any account in this \
                     wallet.",
                    proof_contents.claim_proof.burn_public_key,
                ))
            })?;

        let resp = execute_claim_burn(
            &self.sdk,
            &self.transaction_service,
            &account,
            proof_contents.clone(),
            1,
            max_epoch,
            true,
            Some(file_name.to_string()),
        )
        .await
        .map_err(ClaimError::Permanent)?;

        let Some(result) = resp.dry_run_result else {
            return Err(ClaimError::Permanent(anyhow::anyhow!(
                "NEVER HAPPEN: Dry run failed for '{}', cannot submit claim: no result returned for dry run.",
                file_name,
            )));
        };
        if let Some(reason) = result.finalize.any_reject() {
            // A burn whose L1 block validators have not yet synced surfaces here as a generic
            // execution failure carrying the verifier's marker phrase. Defer these so the claim
            // lands once the base layer catches up, rather than dropping it as permanent.
            if is_burn_not_yet_claimable(&reason.to_string()) {
                return Err(ClaimError::Deferred);
            }
            return Err(ClaimError::Permanent(anyhow::anyhow!(
                "Dry run rejected for '{}': {}. Cannot submit claim.",
                file_name,
                reason,
            )));
        }
        let Some(required_fees) = resp.required_fees else {
            return Err(ClaimError::Permanent(anyhow::anyhow!(
                "NEVER HAPPEN: Dry run failed for '{}', cannot submit claim: no required fees for dry run.",
                file_name,
            )));
        };

        // Delegate all crypto work and submission to the shared handler function.
        // Any error here is treated as permanent: by this point the file is readable and the account
        // is known, so failures indicate a bad proof (wrong key, ownership check failed, fee too high,
        // corrupt encrypted data). Retrying would produce the same result.
        let resp = execute_claim_burn(
            &self.sdk,
            &self.transaction_service,
            &account,
            proof_contents,
            required_fees,
            max_epoch,
            false,
            Some(file_name.to_string()),
        )
        .await
        .map_err(ClaimError::Permanent)?;

        Ok(resp.transaction_id)
    }

    async fn query_current_epoch(&self) -> anyhow::Result<Epoch> {
        self.sdk
            .get_network_interface()
            .get_current_epoch()
            .await
            .map_err(Into::into)
    }
}

struct PendingClaim {
    /// The L1 epoch the burn must be strictly past before it is claimable, read from the proof
    /// file's `mined_in_epoch`. `None` until resolved on an interval check; a proof lacking the
    /// field resolves to `Epoch(0)` (attempt immediately, deferring via the dry-run backstop).
    claim_after_epoch: Option<Epoch>,
    /// Number of transient errors encountered so far. The per-error `max_retries` limit determines
    /// when the claim is dropped from the queue (the file remains on disk for manual retry).
    retries: u32,
    /// Number of times submission was deferred because the burn's L1 block is not yet synced.
    /// The claim is dropped once this reaches `MAX_RETRIES_DEFERRED`.
    deferrals: u32,
}

impl PendingClaim {
    fn new() -> Self {
        Self {
            claim_after_epoch: None,
            retries: 0,
            deferrals: 0,
        }
    }
}

/// Categorises errors to determine retry behaviour for auto-claims.
/// The validity window stamped on claim transactions this service builds. Claims are submitted
/// unattended, so the window only has to cover the submission itself. Derived from the epoch the
/// caller already resolved: re-querying it here would turn a momentary indexer outage into a
/// permanent claim failure.
fn claim_max_epoch(current_epoch: Epoch) -> Epoch {
    Epoch(current_epoch.as_u64().saturating_add(CLAIM_TRANSACTION_VALIDITY_EPOCHS))
}

enum ClaimError {
    /// A permanent error (account not in this wallet, invalid proof data). Remove from queue; the
    /// proof file remains in `burn_proof_dir` for the user to inspect and retry manually.
    Permanent(anyhow::Error),
    /// A transient error (network unavailable, indexer unreachable, file still being written).
    /// Leave in queue and retry on the next epoch check interval, up to `max_retries` times.
    Transient { error: anyhow::Error, max_retries: u32 },
    /// The burn is valid but not yet claimable: validators have not synced the L1 block containing
    /// the burn into an epoch at or before the current L2 epoch. Keep the claim queued and re-check
    /// each interval, up to `MAX_RETRIES_DEFERRED` times, then give up (the file stays for a manual claim).
    Deferred,
}

impl ClaimError {
    fn transient(error: anyhow::Error, max_retries: u32) -> Self {
        Self::Transient { error, max_retries }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defers_burn_not_yet_synced_rejection() {
        // Mirrors the claim-burn verifier's rejection when the L1 block is not yet synced.
        let reason = "Execution failure: At instruction #0: Invalid burn claim proof: block header not found for hash \
                      0a1b2c. The claim may be invalid, or the burn may have occurred after the current epoch, and \
                      therefore is not yet claimable.";
        assert!(is_burn_not_yet_claimable(reason));
    }

    #[test]
    fn does_not_defer_other_rejections() {
        assert!(!is_burn_not_yet_claimable(
            "Execution failure: At instruction #0: Insufficient funds"
        ));
        assert!(!is_burn_not_yet_claimable("Failed to lock inputs: input conflict"));
    }

    #[test]
    fn claim_after_epoch_uses_mined_epoch_else_zero() {
        // A known mined epoch gates on `current > mined`, i.e. the claim lands one epoch later.
        assert_eq!(claim_after_epoch(Some(5)), Epoch(5));
        // No mined epoch (older L1 proof) => attempt immediately; the dry-run backstop defers it.
        assert_eq!(claim_after_epoch(None), Epoch(0));
    }
}
