//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_consensus_types::{HighTc, SignedMessage, TcSignatures, TimeoutCertificate, TimeoutVote, Vote};
use tari_ootle_common_types::{Epoch, NodeHeight, committee::Committee};
use tari_ootle_storage::StateStore;
use tari_ootle_transaction::Network;
use tari_sidechain::SignerBitmap;

use super::collector::VoteCollector;
use crate::{
    hotstuff::{
        epoch_state::EpochState,
        error::HotStuffError,
        vote_collector::{collector::exceeds_vote_lookahead, helpers::check_eligibility},
    },
    tracing::TraceTimer,
    traits::{CertificateStore, ConsensusSpec, ValidatorSignatureVerifierService},
    validations::signed_vote::SignedTimeoutVote,
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::timeout_collector";

#[derive(Clone)]
pub struct TimeoutVoteCollector<TConsensusSpec: ConsensusSpec> {
    network: Network,
    vote_collector: VoteCollector<TimeoutVote>,
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    vote_signer_service: TConsensusSpec::SignerService,
}

impl<TConsensusSpec> TimeoutVoteCollector<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        network: Network,
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        vote_signer_service: TConsensusSpec::SignerService,
    ) -> Self {
        Self {
            network,
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
                let new_tc =
                    create_timeout_certificate(epoch_state.local_committee(), current_epoch, height, &quorum_votes)?;
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

        let signed_vote = SignedTimeoutVote {
            message: vote.to_message(self.network),
            signature: &vote.signature,
        };
        if !self.vote_signer_service.verify(&signed_vote) {
            return Err(HotStuffError::InvalidVoteSignature {
                signer_public_key: *vote.public_key(),
            });
        }

        Ok(())
    }
}

/// The certificate for `votes`, each a verified timeout vote by a member of `committee`.
fn create_timeout_certificate<TAddr: PartialEq>(
    committee: &Committee<TAddr>,
    epoch: Epoch,
    height: NodeHeight,
    votes: &[TimeoutVote],
) -> Result<TimeoutCertificate, HotStuffError> {
    let mut signers = votes
        .iter()
        .map(|vote| {
            let index = committee.index_of_public_key(vote.public_key()).ok_or_else(|| {
                HotStuffError::InvariantError(format!(
                    "Timeout vote signer {} is not a member of the committee",
                    vote.public_key()
                ))
            })?;
            Ok((index, vote.as_signed_timeout()))
        })
        .collect::<Result<Vec<_>, HotStuffError>>()?;
    signers.sort_by_key(|(index, _)| *index);
    let bitmap = SignerBitmap::from_indices(committee.len(), signers.iter().map(|(index, _)| *index))
        .map_err(|e| HotStuffError::InvariantError(format!("Timeout vote signers do not index the committee: {e}")))?;
    let signatures = TcSignatures::Schnorr {
        signers: bitmap,
        timeouts: signers.into_iter().map(|(_, timeout)| timeout).collect(),
    };
    Ok(TimeoutCertificate::new(epoch, height, signatures))
}

#[cfg(test)]
mod tests {
    use tari_consensus_types::{SignedTimeout, ValidatorSignatureBytes};
    use tari_ootle_common_types::{VotePower, committee::CommitteeMember};
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

    fn committee(bytes: &[u8]) -> Committee<RistrettoPublicKeyBytes> {
        bytes
            .iter()
            .map(|b| CommitteeMember {
                address: public_key(*b),
                public_key: public_key(*b),
                vote_power: VotePower::of(1),
            })
            .collect()
    }

    #[test]
    fn it_indexes_signers_by_committee_position() {
        let committee = committee(&[4, 2, 3, 1]);
        let orders = [[3, 1, 2], [1, 2, 3], [2, 3, 1]];
        let certificates = orders
            .iter()
            .map(|order| {
                let votes = order.iter().map(|b| vote(*b)).collect::<Vec<_>>();
                create_timeout_certificate(&committee, Epoch(1), NodeHeight(5), &votes).unwrap()
            })
            .collect::<Vec<_>>();

        let expected = TcSignatures::Schnorr {
            signers: SignerBitmap::from_indices(4, [0, 1, 2]).unwrap(),
            timeouts: [1, 2, 3]
                .iter()
                .map(|b| SignedTimeout {
                    high_pc_height: NodeHeight(*b),
                    signature: SchnorrSignatureBytes::zero(),
                })
                .collect(),
        };
        for certificate in &certificates {
            assert_eq!(*certificate.signatures(), expected);
            certificate.signatures().check_well_formed(committee.len()).unwrap();
            assert_eq!(certificate.calculate_id(), certificates[0].calculate_id());
        }
    }

    #[test]
    fn it_rejects_a_signer_outside_the_committee() {
        let err = create_timeout_certificate(&committee(&[1, 2, 3]), Epoch(1), NodeHeight(5), &[vote(1), vote(5)])
            .unwrap_err();
        assert!(matches!(err, HotStuffError::InvariantError(_)), "{err}");
    }
}
