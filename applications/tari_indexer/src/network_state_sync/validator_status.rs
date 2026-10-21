//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, hash_map::Entry},
    sync::Arc,
    time::SystemTime,
};

use log::*;
use ootle_network::Network;
use tari_consensus::hotstuff::ConsensusCurrentState;
use tari_epoch_manager::{EpochManagerReader, service::EpochManagerHandle};
use tari_ootle_common_types::{Epoch, NodeHeight, ShardGroup};
use tari_ootle_p2p::{PeerAddress, proto::rpc};
use tari_ootle_storage::consensus_models::{CommittedBlockProof, VerifiedBlockTip};
use tokio::sync::RwLock;

use crate::network_state_sync::committee_client::ValidatorRpcSession;

const LOG_TARGET: &str = "tari::indexer::network_state_sync::validator_status";

/// A snapshot of one validator's self-reported consensus pacemaker state, observed by the indexer
/// during a recent sync round.
///
/// This is diagnostic only and intentionally *unverified* - it backs the Validators page and is not
/// trusted for sync-source selection. The trust decision is made separately by verifying the peer's
/// committed block proof (see [`ValidatorStatusMonitor::probe`]).
#[derive(Debug, Clone)]
pub struct ValidatorStatusSnapshot {
    pub epoch: Epoch,
    pub height: NodeHeight,
    pub state: ConsensusCurrentState,
    pub observed_at: SystemTime,
}

/// The outcome of the most recent probe of one validator, alongside the last snapshot any probe of
/// it produced.
#[derive(Debug, Clone)]
pub struct ValidatorProbeRecord {
    pub shard_group: ShardGroup,
    pub probed_at: SystemTime,
    /// Why the most recent probe produced no snapshot, or `None` when it produced `snapshot`.
    pub error: Option<ProbeFailure>,
    /// The last snapshot a probe produced. It predates `probed_at` whenever `error` is set.
    pub snapshot: Option<ValidatorStatusSnapshot>,
}

impl ValidatorProbeRecord {
    fn from_outcome(
        shard_group: ShardGroup,
        probed_at: SystemTime,
        outcome: Result<ValidatorStatusSnapshot, ProbeFailure>,
    ) -> Self {
        let (snapshot, error) = match outcome {
            Ok(snapshot) => (Some(snapshot), None),
            Err(e) => (None, Some(e)),
        };
        Self {
            shard_group,
            probed_at,
            error,
            snapshot,
        }
    }

    /// Records a probe's outcome. A failed probe keeps the previous snapshot, now stale.
    fn apply(
        &mut self,
        shard_group: ShardGroup,
        probed_at: SystemTime,
        outcome: Result<ValidatorStatusSnapshot, ProbeFailure>,
    ) {
        self.shard_group = shard_group;
        self.probed_at = probed_at;
        match outcome {
            Ok(snapshot) => {
                self.error = None;
                self.snapshot = Some(snapshot);
            },
            Err(e) => self.error = Some(e),
        }
    }
}

/// Why a probe of a validator produced no snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeFailure {
    /// The validator did not return a usable consensus state.
    StatusUnavailable(String),
    /// The validator served a committed block proof that failed verification, so it is not
    /// trusted as a sync source.
    InvalidProof(String),
}

/// In-memory, lazily-populated map of the latest probe of each validator the indexer has contacted.
#[derive(Clone)]
pub struct ValidatorStatusMonitor {
    network: Network,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    inner: Arc<RwLock<HashMap<PeerAddress, ValidatorProbeRecord>>>,
}

impl ValidatorStatusMonitor {
    pub fn new(network: Network, epoch_manager: EpochManagerHandle<PeerAddress>) -> Self {
        Self {
            network,
            epoch_manager,
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn records(&self) -> Vec<(PeerAddress, ValidatorProbeRecord)> {
        let guard = self.inner.read().await;
        guard.iter().map(|(k, v)| (*k, v.clone())).collect()
    }

    /// Probe a validator before trusting it as a sync source.
    ///
    /// The meaningful check is the verification of the peer's latest committed block proof against
    /// the shard group committee: a forged or malformed proof yields [`ProbeError::InvalidProof`] so
    /// callers can refuse to sync from that peer. Other verification failures are non-fatal (e.g.
    /// the peer has nothing committed yet).
    ///
    /// As a side effect, the peer's self-reported consensus state is recorded for the diagnostic
    /// Validators page. That status is deliberately *not* trusted for sync-source selection.
    pub async fn probe(
        &self,
        session: &mut ValidatorRpcSession,
        shard_group: ShardGroup,
    ) -> Result<Option<VerifiedBlockTip>, ProbeError> {
        let peer = *session.peer_address();

        // The trust decision: verify how far the peer has actually committed. A forged proof gates
        // the peer out; other failures are tolerated. On success the verified tip is returned so the
        // caller can record the quorum-signed state root.
        let verified_tip = match self.verify_committed_tip(session).await {
            Ok(tip) => Some(tip),
            Err(e) => {
                if e.is_invalid_proof() {
                    self.record(peer, shard_group, Err(ProbeFailure::InvalidProof(e.to_string())))
                        .await;
                    return Err(e);
                }
                debug!(target: LOG_TARGET, "Could not verify committed tip for validator {peer}: {e}");
                None
            },
        };

        // Unverified consensus status for diagnostics. A failure here does not disqualify the peer;
        // it is recorded against the peer so the Validators page can show why its snapshot is stale.
        let snapshot = Self::fetch_consensus_state(session).await;
        match &snapshot {
            Ok(snapshot) => debug!(
                target: LOG_TARGET,
                "Observed validator {peer} in {shard_group}: epoch={}, height={}, state={}",
                snapshot.epoch, snapshot.height, snapshot.state
            ),
            Err(e) => warn!(target: LOG_TARGET, "Consensus state probe failed for validator {peer}: {e}"),
        }
        self.record(peer, shard_group, snapshot.map_err(ProbeFailure::StatusUnavailable))
            .await;
        Ok(verified_tip)
    }

    async fn fetch_consensus_state(session: &mut ValidatorRpcSession) -> Result<ValidatorStatusSnapshot, String> {
        let resp = session
            .get_consensus_state(rpc::GetConsensusStateRequest {})
            .await
            .map_err(|e| format!("consensus state request failed: {e}"))?;
        let epoch = resp
            .epoch
            .map(Epoch::from)
            .ok_or_else(|| "consensus state response is missing epoch".to_string())?;
        let state = rpc::ConsensusState::try_from(resp.state).map_err(|e| {
            format!(
                "consensus state response has invalid state discriminant ({}): {e}",
                resp.state
            )
        })?;
        Ok(ValidatorStatusSnapshot {
            epoch,
            height: NodeHeight::from(resp.height),
            state: ConsensusCurrentState::from(state),
            observed_at: SystemTime::now(),
        })
    }

    async fn record(
        &self,
        peer: PeerAddress,
        shard_group: ShardGroup,
        outcome: Result<ValidatorStatusSnapshot, ProbeFailure>,
    ) {
        let now = SystemTime::now();
        match self.inner.write().await.entry(peer) {
            Entry::Occupied(entry) => entry.into_mut().apply(shard_group, now, outcome),
            Entry::Vacant(entry) => {
                entry.insert(ValidatorProbeRecord::from_outcome(shard_group, now, outcome));
            },
        }
    }

    /// Fetch and verify the validator's latest committed block proof against its shard group
    /// committee. Verification is the basis for deciding whether to trust the peer as a sync source.
    async fn verify_committed_tip(&self, session: &mut ValidatorRpcSession) -> Result<VerifiedBlockTip, ProbeError> {
        let resp = session
            .get_committed_block_proof(rpc::GetCommittedBlockProofRequest {})
            .await
            .map_err(|e| ProbeError::Rpc(e.to_string()))?;

        let proof = CommittedBlockProof::from_bytes(&resp.commit_proof)
            .map_err(|e| ProbeError::InvalidProof(format!("undecodable commit proof: {e}")))?;

        let epoch = proof.epoch();
        let proof_shard_group = proof
            .shard_group()
            .map_err(|e| ProbeError::InvalidProof(format!("invalid shard group in proof header: {e}")))?;

        // The committee for the proof's epoch/shard group is what is expected to have signed it.
        // Verifying against the wrong committee will (correctly) fail the quorum check below.
        let committee = self
            .epoch_manager
            .get_committee_by_shard_group(epoch, proof_shard_group)
            .await
            .map_err(|e| ProbeError::CommitteeUnavailable(e.to_string()))?;

        proof
            .validate(self.network, committee.quorum_threshold(), &*committee)
            .map_err(|e| ProbeError::InvalidProof(e.to_string()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("RPC error: {0}")]
    Rpc(String),
    #[error("committee unavailable for proof epoch/shard group: {0}")]
    CommitteeUnavailable(String),
    #[error("validator served an invalid committed block proof: {0}")]
    InvalidProof(String),
}

impl ProbeError {
    /// True when the peer served a cryptographically invalid (forged or malformed) proof, as
    /// opposed to merely being unreachable or not yet having anything committed.
    pub fn is_invalid_proof(&self) -> bool {
        matches!(self, ProbeError::InvalidProof(_))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn shard_group() -> ShardGroup {
        ShardGroup::new(1u32, 256u32)
    }

    fn unavailable() -> ProbeFailure {
        ProbeFailure::StatusUnavailable("consensus state request failed".to_string())
    }

    fn snapshot(height: u64, observed_at: SystemTime) -> ValidatorStatusSnapshot {
        ValidatorStatusSnapshot {
            epoch: Epoch(7),
            height: NodeHeight::from(height),
            state: ConsensusCurrentState::Running,
            observed_at,
        }
    }

    #[test]
    fn a_failed_probe_keeps_the_last_snapshot_and_records_the_error() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let t1 = t0 + Duration::from_secs(60);
        let mut record = ValidatorProbeRecord::from_outcome(shard_group(), t0, Ok(snapshot(10, t0)));

        record.apply(shard_group(), t1, Err(unavailable()));

        assert_eq!(record.probed_at, t1);
        assert_eq!(record.error, Some(unavailable()));
        let snapshot = record.snapshot.as_ref().unwrap();
        assert_eq!(snapshot.height, NodeHeight::from(10));
        assert_eq!(snapshot.observed_at, t0);
    }

    #[test]
    fn a_successful_probe_clears_the_error() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let t1 = t0 + Duration::from_secs(60);
        let mut record = ValidatorProbeRecord::from_outcome(shard_group(), t0, Err(unavailable()));
        assert!(record.snapshot.is_none());

        record.apply(shard_group(), t1, Ok(snapshot(11, t1)));

        assert_eq!(record.error, None);
        assert_eq!(record.snapshot.as_ref().unwrap().height, NodeHeight::from(11));
    }
}
