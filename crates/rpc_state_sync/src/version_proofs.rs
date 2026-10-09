//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use anyhow::anyhow;
use tari_epoch_manager::EpochManagerReader;
use tari_ootle_common_types::{Epoch, VotePower};
use tari_ootle_p2p::PeerAddress;
use tari_ootle_storage::consensus_models::{CommittedBlockProof, STATE_VERSION_PROOF_INTERVAL};
use tari_state_tree::Version;

use crate::error::RpcStateSyncError;

/// Validates a commit proof's quorum certificates against the committee of its block's epoch and shard group.
pub trait CommitProofValidator {
    fn validate(
        &self,
        commit_proof: &CommittedBlockProof,
    ) -> impl Future<Output = Result<(), RpcStateSyncError>> + Send;
}

/// Validates commit proofs against the committees the epoch manager records, for blocks no later than the epoch
/// of the checkpoint being synced to.
pub(crate) struct CommitteeProofValidator<'a, TEpochManager> {
    epoch_manager: &'a TEpochManager,
    max_epoch: Epoch,
}

impl<'a, TEpochManager> CommitteeProofValidator<'a, TEpochManager> {
    pub fn new(epoch_manager: &'a TEpochManager, max_epoch: Epoch) -> Self {
        Self {
            epoch_manager,
            max_epoch,
        }
    }
}

impl<TEpochManager> CommitProofValidator for CommitteeProofValidator<'_, TEpochManager>
where TEpochManager: EpochManagerReader<Addr = PeerAddress> + Sync
{
    async fn validate(&self, commit_proof: &CommittedBlockProof) -> Result<(), RpcStateSyncError> {
        let epoch = commit_proof.epoch();
        if epoch > self.max_epoch {
            return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                "State version proof is for a block at epoch {epoch}, after the checkpoint epoch {}",
                self.max_epoch
            )));
        }
        let shard_group = commit_proof
            .shard_group()
            .map_err(|e| RpcStateSyncError::InvalidResponse(e.into()))?;
        let committee = self
            .epoch_manager
            .get_committee_by_shard_group(epoch, shard_group)
            .await?;
        commit_proof
            .validate(committee.quorum_threshold(), |pk| {
                Ok(committee.get_power_by_public_key(pk).unwrap_or_else(VotePower::zero))
            })
            .map_err(|e| RpcStateSyncError::InvalidResponse(anyhow!("Invalid commit proof: {e}")))?;
        Ok(())
    }
}

/// The first state version at or after `version` that a stream must prove.
fn proof_point_at_or_after(version: Version) -> Version {
    version.max(1).next_multiple_of(STATE_VERSION_PROOF_INTERVAL)
}

/// Tracks which proof points a shard's stream still owes, and rejects the stream once it moves past one without
/// proving it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProofSchedule {
    /// Whether the stream must prove every proof point. A network launched before streams carried proofs holds
    /// history that no node can prove, so there a stream is held to the schedule only from its first proof on.
    enforced: bool,
    /// The next proof point the stream owes.
    next_point: Version,
    /// The last version the stream covers.
    last: Version,
}

impl ProofSchedule {
    pub fn new(enforced_from_start: bool, start_version: Version, last: Version) -> Self {
        Self {
            enforced: enforced_from_start,
            next_point: proof_point_at_or_after(start_version),
            last,
        }
    }

    /// Whether the stream is held to the schedule.
    pub fn is_enforced(&self) -> bool {
        self.enforced
    }

    /// Rejects a batch at `state_version` if the stream owes a proof for an earlier version.
    pub fn check_batch(&self, state_version: Version) -> Result<(), RpcStateSyncError> {
        if self.enforced && self.next_point < state_version && self.next_point <= self.last {
            return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                "Peer streamed v{state_version} without proving v{}",
                self.next_point
            )));
        }
        Ok(())
    }

    /// Records a verified proof of `state_version`.
    pub fn proven(&mut self, state_version: Version) {
        self.enforced = true;
        if state_version >= self.next_point {
            self.next_point = proof_point_at_or_after(state_version.saturating_add(1));
        }
    }

    /// Rejects a completed stream that still owes a proof.
    pub fn check_complete(&self) -> Result<(), RpcStateSyncError> {
        if self.enforced && self.next_point <= self.last {
            return Err(RpcStateSyncError::InvalidResponse(anyhow!(
                "Peer completed the stream without proving v{}",
                self.next_point
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: Version = STATE_VERSION_PROOF_INTERVAL;

    #[test]
    fn proof_points_are_multiples_of_the_interval() {
        assert_eq!(proof_point_at_or_after(0), N);
        assert_eq!(proof_point_at_or_after(1), N);
        assert_eq!(proof_point_at_or_after(N), N);
        assert_eq!(proof_point_at_or_after(N + 1), 2 * N);
    }

    #[test]
    fn a_stream_may_reach_a_proof_point_before_proving_it() {
        let schedule = ProofSchedule::new(true, 1, 3 * N);
        schedule.check_batch(N).unwrap();
        assert!(schedule.check_batch(N + 1).is_err());
    }

    #[test]
    fn a_proof_moves_the_schedule_to_the_next_point() {
        let mut schedule = ProofSchedule::new(true, 1, 3 * N);
        schedule.proven(N);
        schedule.check_batch(2 * N).unwrap();
        assert!(schedule.check_batch(2 * N + 1).is_err());
        assert!(schedule.check_complete().is_err());
        schedule.proven(2 * N);
        schedule.proven(3 * N);
        schedule.check_complete().unwrap();
    }

    #[test]
    fn points_beyond_the_stream_are_not_owed() {
        let schedule = ProofSchedule::new(true, 1, N - 1);
        schedule.check_batch(N - 1).unwrap();
        schedule.check_complete().unwrap();
    }

    #[test]
    fn an_unenforced_schedule_is_enforced_from_its_first_proof() {
        let mut schedule = ProofSchedule::new(false, 1, 3 * N);
        schedule.check_batch(2 * N + 1).unwrap();
        schedule.proven(2 * N);
        schedule.check_batch(3 * N).unwrap();
        assert!(schedule.check_complete().is_err());
    }
}
