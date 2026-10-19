//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_consensus_types::{HighTc, SignedMessage, TimeoutCertificate, TimeoutVote, Vote};
use tari_ootle_common_types::{Epoch, NodeHeight};
use tari_ootle_storage::StateStore;

use super::collector::VoteCollector;
use crate::{
    hotstuff::{
        epoch_state::EpochState,
        error::HotStuffError,
        vote_collector::{collector::exceeds_vote_lookahead, helpers::check_eligibility},
    },
    tracing::TraceTimer,
    traits::{CertificateStore, ConsensusSpec, ValidatorSignatureVerifierService},
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::timeout_collector";

#[derive(Clone)]
pub struct TimeoutVoteCollector<TConsensusSpec: ConsensusSpec> {
    vote_collector: VoteCollector<TimeoutVote>,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    vote_signer_service: TConsensusSpec::SignerService,
}

impl<TConsensusSpec> TimeoutVoteCollector<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        vote_signer_service: TConsensusSpec::SignerService,
    ) -> Self {
        Self {
            store,
            vote_collector: VoteCollector::new(),
            epoch_manager,
            vote_signer_service,
        }
    }

    /// Returns Some if quorum is reached
    pub async fn check_and_collect_vote(
        &self,
        from: TConsensusSpec::Addr,
        current_height: NodeHeight,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        vote: TimeoutVote,
    ) -> Result<Option<(TimeoutCertificate, HighTc)>, HotStuffError> {
        let _timer = TraceTimer::debug(LOG_TARGET, "check_and_collect_vote (TimeoutVote)");
        debug!(
            target: LOG_TARGET,
            "📬 Validating timeout vote message from {from}: {vote}"
        );

        let current_epoch = epoch_state.epoch();
        let local_committee_info = epoch_state.local_committee_info();

        // Before any signature verification: a vote this far ahead is dropped by the collector, so the
        // verification would be work done for nothing.
        if exceeds_vote_lookahead(current_height, vote.height) {
            warn!(
                target: LOG_TARGET,
                "🗑️ Discarding {} from {}: it is too far ahead of our current view {}/{}",
                vote,
                from,
                current_epoch,
                current_height
            );
            return Ok(None);
        }

        let sender_vn =
            check_eligibility::<TConsensusSpec, _>(&self.epoch_manager, from, &vote, local_committee_info).await?;
        self.validate_vote(current_epoch, &vote)?;
        let height = vote.height;

        let result = self
            .vote_collector
            .collect_vote(
                &sender_vn,
                current_epoch,
                current_height,
                vote,
                epoch_state.local_committee(),
            )
            .await;

        match result {
            Ok(Some((quorum_votes, _))) => {
                let new_tc = create_timeout_certificate(current_epoch, height, &quorum_votes);
                let high_tc = self.store.with_write_tx(|tx| new_tc.update_highest(tx))?;
                if new_tc.calculate_id() == *high_tc.id() {
                    info!(target: LOG_TARGET, "🕒️ New HIGH {}", new_tc);
                } else {
                    info!(target: LOG_TARGET, "❓️ New TC from votes {} but it is not the highest TC {}", new_tc, high_tc);
                }

                Ok(Some((new_tc, high_tc)))
            },
            Ok(None) => {
                debug!(target: LOG_TARGET, "🟡 No quorum reached yet for TimeoutVote at height {}", height);
                Ok(None)
            },
            Err(duplicate) => {
                // Not equivocation, and no evidence can be built from it: a timeout vote signs
                // (epoch, height) and nothing else, which is the view it is bucketed under, so the
                // two votes necessarily attest to the same thing and differ only in the signature's
                // nonce. Re-signing one view is something an honest node does, for instance after a
                // restart loses the in-memory record of the NEWVIEW it last sent.
                warn!(target: LOG_TARGET, "❌ {}", duplicate);
                Ok(None)
            },
        }
    }

    fn validate_vote(&self, current_epoch: Epoch, vote: &TimeoutVote) -> Result<(), HotStuffError> {
        if current_epoch != vote.epoch() {
            return Err(HotStuffError::InvalidVote {
                signer_public_key: vote.signature.public_key,
                details: format!(
                    "Our current view is at epoch {} but the vote was for epoch {}",
                    current_epoch,
                    vote.epoch()
                ),
            });
        }

        if !self.vote_signer_service.verify(vote) {
            return Err(HotStuffError::InvalidVoteSignature {
                signer_public_key: *vote.public_key(),
            });
        }

        Ok(())
    }
}

/// Timeouts are ordered by signer public key so that a node builds the same certificate from the same votes. The order
/// is applied here rather than in `TimeoutCertificate::new`, which also decodes received certificates: those must keep
/// the order their proposer committed to, or their recomputed ids would differ from the signed ones.
fn create_timeout_certificate(epoch: Epoch, height: NodeHeight, votes: &[TimeoutVote]) -> TimeoutCertificate {
    let mut timeouts = votes.iter().map(|vote| vote.as_signed_timeout()).collect::<Vec<_>>();
    timeouts.sort_by(|a, b| a.signature.public_key().cmp(b.signature.public_key()));
    TimeoutCertificate::new(epoch, height, timeouts)
}

#[cfg(test)]
mod tests {
    use tari_consensus_types::ValidatorSignatureBytes;
    use tari_template_lib_types::crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes};

    use super::*;

    fn public_key(byte: u8) -> RistrettoPublicKeyBytes {
        RistrettoPublicKeyBytes::from_bytes(&[byte; 32]).unwrap()
    }

    fn vote(byte: u8) -> TimeoutVote {
        TimeoutVote {
            epoch: Epoch(1),
            height: NodeHeight(5),
            high_pc_height: NodeHeight(u64::from(byte)),
            signature: ValidatorSignatureBytes::new(public_key(byte), SchnorrSignatureBytes::zero()),
        }
    }

    #[test]
    fn it_orders_timeouts_by_public_key() {
        let orders = [[3, 1, 2], [1, 2, 3], [2, 3, 1]];
        let certificates = orders
            .iter()
            .map(|order| {
                let votes = order.iter().map(|b| vote(*b)).collect::<Vec<_>>();
                create_timeout_certificate(Epoch(1), NodeHeight(5), &votes)
            })
            .collect::<Vec<_>>();

        for certificate in &certificates {
            let public_keys = certificate
                .timeouts()
                .iter()
                .map(|t| *t.signature.public_key())
                .collect::<Vec<_>>();
            assert_eq!(public_keys, [public_key(1), public_key(2), public_key(3)]);
            assert_eq!(certificate.calculate_id(), certificates[0].calculate_id());
        }
    }
}
